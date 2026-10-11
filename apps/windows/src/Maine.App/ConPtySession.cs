using System.Runtime.InteropServices;
using System.Text;

namespace Maine.App;

/// <summary>
/// Runs <c>ssh.exe</c> attached to a Windows pseudo console (ConPTY) and
/// pumps bytes in both directions. This is the Windows counterpart of the
/// macOS <c>forkpty</c> + <c>LocalProcessTerminalView</c> pairing: the child
/// gets a real terminal, so interactive prompts, resizing and Ctrl/Esc
/// sequences behave exactly as in a terminal window.
/// </summary>
public sealed class ConPtySession : IDisposable
{
    public event Action<byte[]>? DataReceived;
    public event Action<int>? Exited;

    private IntPtr _pseudoConsole = IntPtr.Zero;
    private IntPtr _processHandle = IntPtr.Zero;
    private IntPtr _inputWrite = IntPtr.Zero;
    private IntPtr _outputRead = IntPtr.Zero;
    private Thread? _readerThread;
    private Thread? _waitThread;
    private bool _disposed;

    public bool IsRunning => _processHandle != IntPtr.Zero;

    public void Start(string executable, IEnumerable<string> arguments,
        IDictionary<string, string> environment, short cols, short rows)
    {
        if (!CreatePipe(out IntPtr inputRead, out _inputWrite, IntPtr.Zero, 0))
            ThrowLastError("CreatePipe (stdin)");
        if (!CreatePipe(out _outputRead, out IntPtr outputWrite, IntPtr.Zero, 0))
        {
            CloseHandle(inputRead);
            ThrowLastError("CreatePipe (stdout)");
        }

        var size = new COORD { X = cols, Y = rows };
        int hr = CreatePseudoConsole(size, inputRead, outputWrite, 0, out _pseudoConsole);
        // The pseudo console owns its side of the pipes from here on.
        CloseHandle(inputRead);
        CloseHandle(outputWrite);
        if (hr != 0)
            throw new InvalidOperationException($"CreatePseudoConsole failed (HRESULT 0x{hr:X8}).");

        IntPtr attrList = IntPtr.Zero;
        try
        {
            int attrSize = 0;
            InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref attrSize);
            attrList = Marshal.AllocHGlobal(attrSize);
            if (!InitializeProcThreadAttributeList(attrList, 1, 0, ref attrSize))
                ThrowLastError("InitializeProcThreadAttributeList");
            IntPtr pc = _pseudoConsole;
            if (!UpdateProcThreadAttribute(attrList, IntPtr.Zero,
                    (IntPtr)ProcThreadAttributePseudoConsole,
                    ref pc, (IntPtr)IntPtr.Size, IntPtr.Zero, IntPtr.Zero))
                ThrowLastError("UpdateProcThreadAttribute");

            var startup = new STARTUPINFOEX();
            startup.StartupInfo.cb = Marshal.SizeOf<STARTUPINFOEX>();
            startup.lpAttributeList = attrList;

            var commandLine = new StringBuilder(CommandLine(executable, arguments));
            bool ok = CreateProcess(null, commandLine, IntPtr.Zero, IntPtr.Zero,
                bInheritHandles: false,
                CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                EnvironmentBlock(environment), null,
                ref startup.StartupInfo, out PROCESS_INFORMATION info);
            if (!ok)
                ThrowLastError("CreateProcess");

            _processHandle = info.hProcess;
            CloseHandle(info.hThread);

            _readerThread = new Thread(PumpOutput) { IsBackground = true, Name = "conpty-reader" };
            _readerThread.Start();
            _waitThread = new Thread(WaitForExit) { IsBackground = true, Name = "conpty-wait" };
            _waitThread.Start();
        }
        finally
        {
            if (attrList != IntPtr.Zero)
            {
                DeleteProcThreadAttributeList(attrList);
                Marshal.FreeHGlobal(attrList);
            }
        }
    }

    public static string CommandLine(string executable, IEnumerable<string> arguments)
    {
        var parts = new List<string> { Quote(executable) };
        parts.AddRange(arguments);
        return string.Join(" ", parts);
    }

    private static string Quote(string value) =>
        value.Any(c => c is ' ' or '\t' or '"') ? $"\"{value.Replace("\"", "\\\"")}\"" : value;

    private static IntPtr EnvironmentBlock(IDictionary<string, string> environment)
    {
        // Double-null-terminated Unicode block for CreateProcess.
        var sb = new StringBuilder();
        foreach (var kv in environment.OrderBy(kv => kv.Key, StringComparer.Ordinal))
        {
            sb.Append(kv.Key);
            sb.Append('=');
            sb.Append(kv.Value);
            sb.Append('\0');
        }
        sb.Append('\0');
        byte[] bytes = Encoding.Unicode.GetBytes(sb.ToString());
        IntPtr block = Marshal.AllocHGlobal(bytes.Length);
        Marshal.Copy(bytes, 0, block, bytes.Length);
        return block;
    }

    public void Write(byte[] data)
    {
        if (_inputWrite == IntPtr.Zero || data.Length == 0)
            return;
        uint written = 0;
        WriteFile(_inputWrite, data, (uint)data.Length, ref written, IntPtr.Zero);
    }

    public void Resize(short cols, short rows)
    {
        if (_pseudoConsole != IntPtr.Zero)
            ResizePseudoConsole(_pseudoConsole, new COORD { X = cols, Y = rows });
    }

    /// <summary>Terminates the child without touching any persisted records
    /// (callers delete records first on explicit close).</summary>
    public void Kill()
    {
        if (_processHandle != IntPtr.Zero)
            TerminateProcess(_processHandle, 255);
    }

    private void PumpOutput()
    {
        var buffer = new byte[16 * 1024];
        try
        {
            while (!_disposed)
            {
                bool ok = ReadFile(_outputRead, buffer, (uint)buffer.Length,
                    out uint read, IntPtr.Zero);
                if (!ok || read == 0)
                    break;
                var chunk = new byte[read];
                Buffer.BlockCopy(buffer, 0, chunk, 0, (int)read);
                DataReceived?.Invoke(chunk);
            }
        }
        catch { /* pipe closed during teardown */ }
    }

    private void WaitForExit()
    {
        try
        {
            if (_processHandle != IntPtr.Zero)
            {
                WaitForSingleObject(_processHandle, INFINITE);
                GetExitCodeProcess(_processHandle, out uint code);
                Exited?.Invoke(unchecked((int)code));
            }
        }
        catch { }
    }

    public void Dispose()
    {
        _disposed = true;
        try { Kill(); } catch { }
        if (_processHandle != IntPtr.Zero) { CloseHandle(_processHandle); _processHandle = IntPtr.Zero; }
        if (_pseudoConsole != IntPtr.Zero) { ClosePseudoConsole(_pseudoConsole); _pseudoConsole = IntPtr.Zero; }
        if (_inputWrite != IntPtr.Zero) { CloseHandle(_inputWrite); _inputWrite = IntPtr.Zero; }
        if (_outputRead != IntPtr.Zero) { CloseHandle(_outputRead); _outputRead = IntPtr.Zero; }
        GC.SuppressFinalize(this);
    }

    private static void ThrowLastError(string what) =>
        throw new InvalidOperationException($"{what} failed (Win32 error {Marshal.GetLastWin32Error()}).");

    private const uint EXTENDED_STARTUPINFO_PRESENT = 0x00080000;
    private const uint CREATE_UNICODE_ENVIRONMENT = 0x00000400;
    private const uint INFINITE = 0xFFFFFFFF;
    private const int ProcThreadAttributePseudoConsole = 0x20016;

    [StructLayout(LayoutKind.Sequential)]
    private struct COORD { public short X; public short Y; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct STARTUPINFOEX
    {
        public STARTUPINFO StartupInfo;
        public IntPtr lpAttributeList;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct STARTUPINFO
    {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwCountCharsX, dwCountCharsY, dwFillAttribute, dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput, hStdOutput, hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct PROCESS_INFORMATION
    {
        public IntPtr hProcess, hThread;
        public int dwProcessId, dwThreadId;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern int CreatePseudoConsole(COORD size, IntPtr hInput, IntPtr hOutput, uint dwFlags, out IntPtr phPC);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern int ClosePseudoConsole(IntPtr hPC);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern int ResizePseudoConsole(IntPtr hPC, COORD size);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CreatePipe(out IntPtr hReadPipe, out IntPtr hWritePipe, IntPtr lpPipeAttributes, int nSize);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern bool CreateProcess(string? lpApplicationName, StringBuilder lpCommandLine,
        IntPtr lpProcessAttributes, IntPtr lpThreadAttributes, bool bInheritHandles,
        uint dwCreationFlags, IntPtr lpEnvironment, string? lpCurrentDirectory,
        ref STARTUPINFO lpStartupInfo, out PROCESS_INFORMATION lpProcessInformation);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool InitializeProcThreadAttributeList(IntPtr lpAttributeList, int dwAttributeCount, int dwFlags, ref int lpSize);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool UpdateProcThreadAttribute(IntPtr lpAttributeList, IntPtr dwFlags,
        IntPtr attribute, ref IntPtr lpValue, IntPtr cbSize, IntPtr lpPreviousValue, IntPtr lpReturnSize);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern void DeleteProcThreadAttributeList(IntPtr lpAttributeList);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool ReadFile(IntPtr hFile, byte[] lpBuffer, uint nNumberOfBytesToRead,
        out uint lpNumberOfBytesRead, IntPtr lpOverlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool WriteFile(IntPtr hFile, byte[] lpBuffer, uint nNumberOfBytesToWrite,
        ref uint lpNumberOfBytesWritten, IntPtr lpOverlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr hObject);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool TerminateProcess(IntPtr hProcess, uint uExitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint WaitForSingleObject(IntPtr hHandle, uint dwMilliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetExitCodeProcess(IntPtr hProcess, out uint lpExitCode);
}
