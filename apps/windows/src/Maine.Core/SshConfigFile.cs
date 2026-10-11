namespace Maine.Core;

/// <summary>
/// One <c>Host</c> block found in the user's ssh config. Maine only looks at
/// the directives it needs to build a host record; OpenSSH applies the rest.
/// Mirrors <c>SSHConfigFile.swift</c> in the macOS app.
/// </summary>
public sealed class SshConfigEntry : IEquatable<SshConfigEntry>
{
    public string Pattern { get; set; } = "";
    public string? Hostname { get; set; }
    public string? User { get; set; }
    public int? Port { get; set; }

    public SshConfigEntry() { }

    public SshConfigEntry(string pattern, string? hostname = null, string? user = null, int? port = null)
    {
        Pattern = pattern;
        Hostname = hostname;
        User = user;
        Port = port;
    }

    /// <summary>True when the pattern names exactly one host (no wildcards),
    /// so it can be offered as a Maine host.</summary>
    public static bool IsConnectable(string pattern)
    {
        if (string.IsNullOrEmpty(pattern) || pattern.Any(char.IsWhiteSpace))
            return false;
        return !pattern.Contains('*') && !pattern.Contains('?');
    }

    public bool Equals(SshConfigEntry? other) =>
        other is not null && Pattern == other.Pattern && Hostname == other.Hostname &&
        User == other.User && Port == other.Port;

    public override bool Equals(object? obj) => Equals(obj as SshConfigEntry);
    public override int GetHashCode() => HashCode.Combine(Pattern, Hostname, User, Port);
}

/// <summary>
/// A best-effort reader for the OpenSSH client configuration. It exists so
/// Maine can offer the user's existing hosts when adding a host, and so the UI
/// can show which ssh config stanza a connection comes from. Maine never
/// writes to this file.
/// </summary>
public sealed class SshConfigFile
{
    public List<SshConfigEntry> Entries { get; } = new();

    public SshConfigFile() { }

    public SshConfigFile(IEnumerable<SshConfigEntry> entries)
    {
        Entries.AddRange(entries);
    }

    public static string DefaultPath
    {
        get
        {
            var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            if (string.IsNullOrEmpty(home))
                home = Environment.GetEnvironmentVariable("HOME") ?? "";
            return Path.Combine(home, ".ssh", "config");
        }
    }

    /// <summary>Reads the user's configuration. A missing or unreadable file
    /// is not an error — it just means there is nothing to import.</summary>
    public static SshConfigFile Load(string? path = null)
    {
        var file = new SshConfigFile();
        ParseFile(path ?? DefaultPath, file.Entries,
            new HashSet<string>(StringComparer.OrdinalIgnoreCase), 0);
        return file;
    }

    public SshConfigEntry? EntryFor(string pattern) =>
        Entries.FirstOrDefault(e => string.Equals(e.Pattern, pattern, StringComparison.OrdinalIgnoreCase));

    private static void ParseFile(string path, List<SshConfigEntry> entries,
        HashSet<string> visited, int depth)
    {
        string full;
        try { full = Path.GetFullPath(path); }
        catch { return; }
        // Guard against `Include` cycles and runaway chains.
        if (depth >= 8 || !visited.Add(full))
            return;

        string contents;
        try { contents = File.ReadAllText(full); }
        catch { return; }

        // Patterns of the `Host` block currently being read.
        var active = new List<string>();

        foreach (var rawLine in contents.Split('\n'))
        {
            var tokens = Tokenize(rawLine);
            if (tokens.Count == 0)
                continue;
            var keyword = tokens[0].ToLowerInvariant();
            var values = tokens.Skip(1).ToList();

            switch (keyword)
            {
                case "host":
                    active.Clear();
                    foreach (var pattern in values)
                    {
                        if (pattern.Length == 0)
                            continue;
                        // The same pattern can turn up in several blocks or files.
                        // Record it once, but keep reading it: OpenSSH keeps the
                        // first value it obtains for a keyword, so a later block can
                        // still supply a value an earlier one left out.
                        if (!entries.Any(e => e.Pattern == pattern))
                            entries.Add(new SshConfigEntry(pattern));
                        active.Add(pattern);
                    }
                    break;

                case "hostname":
                    if (values.Count > 0)
                        foreach (var i in ActiveIndices(entries, active))
                            entries[i].Hostname ??= values[0];
                    break;

                case "user":
                    if (values.Count > 0)
                        foreach (var i in ActiveIndices(entries, active))
                            entries[i].User ??= values[0];
                    break;

                case "port":
                    if (values.Count > 0 && int.TryParse(values[0], out int port))
                        foreach (var i in ActiveIndices(entries, active))
                            entries[i].Port ??= port;
                    break;

                case "include":
                    var folder = Path.GetDirectoryName(full) ?? "";
                    foreach (var argument in values)
                        foreach (var included in ExpandInclude(argument, folder))
                            ParseFile(included, entries, visited, depth + 1);
                    break;

                default:
                    // Not needed to build a host record; OpenSSH applies everything else.
                    break;
            }
        }
    }

    /// <summary>Indices of the entries the current <c>Host</c> block talks about.</summary>
    private static List<int> ActiveIndices(List<SshConfigEntry> entries, List<string> patterns) =>
        entries.Select((e, i) => (e, i))
            .Where(t => patterns.Contains(t.e.Pattern))
            .Select(t => t.i)
            .ToList();

    /// <summary>Splits one line into tokens, dropping comments and handling <c>Keyword=value</c>.</summary>
    internal static List<string> Tokenize(string line)
    {
        var tokens = new List<string>();
        var current = new System.Text.StringBuilder();

        foreach (char c in StripComment(line))
        {
            if (c == ' ' || c == '\t' || c == '=')
            {
                if (current.Length > 0)
                {
                    tokens.Add(Unquote(current.ToString()));
                    current.Clear();
                }
            }
            else
            {
                current.Append(c);
            }
        }
        if (current.Length > 0)
            tokens.Add(Unquote(current.ToString()));
        return tokens;
    }

    /// <summary>Drops a trailing <c>#</c> comment. The "#" only opens a comment
    /// at the start of a line or after whitespace, so a value that contains
    /// one survives.</summary>
    internal static string StripComment(string line)
    {
        int hash = line.IndexOf('#');
        if (hash < 0)
            return line;
        if (hash != 0 && !char.IsWhiteSpace(line[hash - 1]))
            return line;
        return line.Substring(0, hash);
    }

    private static string Unquote(string token)
    {
        if (token.Length >= 2 &&
            ((token.StartsWith('"') && token.EndsWith('"')) ||
             (token.StartsWith('\'') && token.EndsWith('\''))))
            return token.Substring(1, token.Length - 2);
        return token;
    }

    /// <summary>Expands <c>~</c>, braces and wildcards in an <c>Include</c>
    /// argument. A relative argument is resolved against the directory of the
    /// file that contained the directive, which is where OpenSSH looks too.</summary>
    internal static List<string> ExpandInclude(string argument, string folder)
    {
        string expanded = ExpandTilde(argument);
        string pattern = Path.IsPathRooted(expanded)
            ? expanded
            : Path.Combine(folder, expanded);

        // Fast path: no glob characters.
        if (pattern.IndexOfAny(new[] { '*', '?', '[' }) < 0)
            return File.Exists(pattern) ? new List<string> { pattern } : new List<string>();

        // Split into a literal directory prefix and a glob remainder so brace
        // expansion and wildcards work without a glob library.
        string dir = Path.GetDirectoryName(pattern) ?? "";
        string filePattern = Path.GetFileName(pattern);
        var matches = new List<string>();
        foreach (var altDir in ExpandBraces(dir))
        {
            if (!Directory.Exists(altDir))
                continue;
            foreach (var file in ExpandBraces(filePattern))
            {
                if (file.IndexOf('[') >= 0)
                {
                    // .NET file patterns do not do character classes; match manually.
                    var regex = new System.Text.RegularExpressions.Regex(
                        "^" + System.Text.RegularExpressions.Regex.Escape(file)
                            .Replace(@"\*", ".*").Replace(@"\?", ".")
                            .Replace(@"\[", "[").Replace(@"\]", "]") + "$",
                        System.Text.RegularExpressions.RegexOptions.IgnoreCase);
                    try
                    {
                        foreach (var candidate in Directory.GetFiles(altDir))
                            if (regex.IsMatch(Path.GetFileName(candidate)))
                                matches.Add(candidate);
                    }
                    catch { /* unreadable directory is not an error */ }
                }
                else
                {
                    try
                    {
                        matches.AddRange(Directory.GetFiles(altDir, file));
                    }
                    catch { /* unreadable directory is not an error */ }
                }
            }
        }
        matches.Sort(StringComparer.Ordinal);
        return matches;
    }

    private static string ExpandTilde(string path)
    {
        if (path == "~" || path.StartsWith("~/") || path.StartsWith("~\\"))
        {
            var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            if (string.IsNullOrEmpty(home))
                home = Environment.GetEnvironmentVariable("HOME") ?? "";
            return home + path.Substring(1);
        }
        return path;
    }

    /// <summary>Expands <c>{a,b}</c> brace groups (including nested ones).</summary>
    internal static List<string> ExpandBraces(string pattern)
    {
        int open = pattern.IndexOf('{');
        if (open < 0)
            return new List<string> { pattern };
        int depth = 0, close = -1;
        for (int i = open; i < pattern.Length; i++)
        {
            if (pattern[i] == '{') depth++;
            else if (pattern[i] == '}')
            {
                depth--;
                if (depth == 0) { close = i; break; }
            }
        }
        if (close < 0)
            return new List<string> { pattern };

        // Split the inside on top-level commas.
        var parts = new List<string>();
        int level = 0, start = open + 1;
        for (int i = open + 1; i < close; i++)
        {
            if (pattern[i] == '{') level++;
            else if (pattern[i] == '}') level--;
            else if (pattern[i] == ',' && level == 0)
            {
                parts.Add(pattern.Substring(start, i - start));
                start = i + 1;
            }
        }
        parts.Add(pattern.Substring(start, close - start));

        string prefix = pattern.Substring(0, open);
        string suffix = pattern.Substring(close + 1);
        var result = new List<string>();
        foreach (var part in parts)
            result.AddRange(ExpandBraces(prefix + part + suffix));
        return result;
    }
}
