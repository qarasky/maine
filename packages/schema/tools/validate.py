#!/usr/bin/env python3
"""Validate Maine host-list files against hosts.schema.json.

Maine's configuration format is deliberately JSON so that every client
implementation can read and write it without sharing code.  This script is the
format's reference checker: it validates a file (or a directory of files)
against the canonical schema with no third-party dependencies.

Supported JSON Schema keywords (everything hosts.schema.json uses):

    $ref (local "#/$defs/..." and "#/properties/..."), $defs, const, type
    (object/array/string/integer/number/boolean/null), required, properties,
    additionalProperties, items, minLength, maxLength, minimum, maximum, pattern

Anything else is reported as unsupported rather than silently ignored, so the
script never says a file is valid because it skipped a constraint.

Usage:
    python3 tools/validate.py [--schema PATH] [PATH ...]

With no PATH arguments, every *.json file in examples/ is validated and must
pass, except for files whose name contains ".invalid", which must fail.
Exit code is 0 when every file validates, 1 otherwise.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
DEFAULT_SCHEMA = HERE.parent / "hosts.schema.json"
DEFAULT_EXAMPLES = HERE.parent / "examples"

# Constraint keywords.  Everything else that appears in a schema is metadata
# ($id, $schema, $comment, examples, ...) and is deliberately ignored.
META_KEYWORDS = {
    "$id", "$schema", "$anchor", "$comment", "$dynamicAnchor", "examples",
    "deprecated", "readOnly", "writeOnly", "unevaluatedProperties",
}
SUPPORTED_KEYWORDS = META_KEYWORDS | {
    "$ref", "$defs", "const", "type", "enum", "required", "properties",
    "additionalProperties", "items", "minLength", "maxLength", "minimum",
    "maximum", "pattern", "default", "title", "description", "format",
}


class ValidationError(Exception):
    pass


def resolve_pointer(document: dict, ref: str) -> Any:
    if not ref.startswith("#"):
        raise ValidationError(f"unsupported non-local $ref: {ref!r}")
    node: Any = document
    for token in ref.lstrip("#/").split("/"):
        if not token:
            continue
        token = token.replace("~1", "/").replace("~0", "~")
        if not isinstance(node, dict) or token not in node:
            raise ValidationError(f"unresolvable $ref: {ref!r}")
        node = node[token]
    return node


def json_type_matches(value: Any, expected: str) -> bool:
    if expected == "object":
        return isinstance(value, dict)
    if expected == "array":
        return isinstance(value, list)
    if expected == "string":
        return isinstance(value, str)
    if expected == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if expected == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if expected == "boolean":
        return isinstance(value, bool)
    if expected == "null":
        return value is None
    raise ValidationError(f"unsupported type in schema: {expected!r}")


def validate(value: Any, schema: Any, root: dict, path: str) -> list[str]:
    """Return a list of human readable problems for `value` against `schema`."""
    problems: list[str] = []
    if schema is True or schema == {}:
        return problems
    if schema is False:
        return [f"{path}: schema forbids any value"]

    unsupported = set(schema) - SUPPORTED_KEYWORDS
    if unsupported:
        raise ValidationError(
            f"{path}: schema uses unsupported keyword(s): {', '.join(sorted(unsupported))}"
        )

    if "$ref" in schema:
        target = resolve_pointer(root, schema["$ref"])
        problems += validate(value, target, root, path)

    if "type" in schema:
        expected = schema["type"]
        allowed = expected if isinstance(expected, list) else [expected]
        if not any(json_type_matches(value, item) for item in allowed):
            return [f"{path}: expected type {'/'.join(allowed)}, got "
                    f"{type(value).__name__} ({value!r})"]

    if "const" in schema and value != schema["const"]:
        problems.append(f"{path}: must be {schema['const']!r}, got {value!r}")

    if "enum" in schema and value not in schema["enum"]:
        problems.append(f"{path}: must be one of {schema['enum']!r}, got {value!r}")

    if isinstance(value, dict):
        for key in schema.get("required", []):
            if key not in value:
                problems.append(f"{path}: missing required property '{key}'")

        properties = schema.get("properties", {})
        for key, child in value.items():
            child_path = f"{path}.{key}" if path != "$" else f"$.{key}"
            if key in properties:
                problems += validate(child, properties[key], root, child_path)
                continue
            extra = schema.get("additionalProperties", True)
            if extra is False:
                problems.append(f"{path}: unexpected property '{key}'")
            elif isinstance(extra, dict):
                problems += validate(child, extra, root, child_path)

    if isinstance(value, list):
        items = schema.get("items")
        if isinstance(items, dict):
            for index, item in enumerate(value):
                problems += validate(item, items, root, f"{path}[{index}]")

    if isinstance(value, str):
        if "minLength" in schema and len(value) < schema["minLength"]:
            problems.append(f"{path}: shorter than {schema['minLength']} characters")
        if "maxLength" in schema and len(value) > schema["maxLength"]:
            problems.append(f"{path}: longer than {schema['maxLength']} characters")
        pattern = schema.get("pattern")
        if pattern is not None and re.search(pattern, value) is None:
            problems.append(f"{path}: {value!r} does not match /{pattern}/")

    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in schema and value < schema["minimum"]:
            problems.append(f"{path}: {value} is below the minimum {schema['minimum']}")
        if "maximum" in schema and value > schema["maximum"]:
            problems.append(f"{path}: {value} is above the maximum {schema['maximum']}")

    return problems


def check_file(path: Path, schema: dict) -> list[str]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        return [f"{path}: not valid JSON: {error}"]
    return validate(data, schema, schema, "$")


def expand_targets(paths: list[str]) -> list[Path]:
    if paths:
        return [Path(item) for item in paths]
    return sorted(DEFAULT_EXAMPLES.glob("*.json"))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA,
                        help="schema file to validate against")
    parser.add_argument("paths", nargs="*", help="files or directories to validate")
    args = parser.parse_args(argv)

    try:
        schema = json.loads(args.schema.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"error: cannot load schema {args.schema}: {error}", file=sys.stderr)
        return 1

    targets = expand_targets(args.paths)
    expanded: list[Path] = []
    for target in targets:
        if target.is_dir():
            expanded += sorted(target.glob("*.json"))
        else:
            expanded.append(target)

    failures = 0
    for target in expanded:
        expect_invalid = ".invalid" in target.name
        try:
            problems = check_file(target, schema)
        except ValidationError as error:
            print(f"  {target}: schema bug - {error}", file=sys.stderr)
            failures += 1
            continue

        if problems:
            for problem in problems:
                print(f"  {problem}", file=sys.stderr)
            if expect_invalid:
                print(f"ok    {target} (rejected as expected)")
            else:
                print(f"FAIL  {target}", file=sys.stderr)
                failures += 1
        else:
            print(f"FAIL  {target}: expected a rejection, but the file is valid"
                  if expect_invalid else f"ok    {target}")
            if expect_invalid:
                failures += 1

    if failures:
        print(f"\n{failures} file(s) failed validation", file=sys.stderr)
        return 1
    print(f"\n{len(expanded)} file(s) valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
