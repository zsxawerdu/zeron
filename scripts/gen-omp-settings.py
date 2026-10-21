#!/usr/bin/env python3
"""Extract OMP settings descriptors from pinned upstream TypeScript source.

OMP declares every setting once via `export const cfgX = register({...})`
in domain `settings.ts` files (see packages/coding-agent/src/config/
registry.ts in can1357/oh-my-pi). The runtime registry is the durable
schema contract (handoff W06: never scrape `omp config list` output,
whose descriptors are incomplete). This generator statically parses
those declarations from a checkout of the PINNED upstream tag and emits
the vendored JSON the Rust side loads.

Computed (non-literal) values are emitted as {"$expr": "<source>"} so
they are visible, never silently fabricated. Only descriptor fields the
settings service needs are kept: id/type/values/default/env/credential
and ui{tab,group,label,description,secret,condition,options}.

Usage:
    python3 scripts/gen-omp-settings.py /path/to/oh-my-pi@v18.12.5 \
        crates/harness/src/omp/settings_descriptors.json
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

DECL = re.compile(r"export\s+const\s+(cfg\w+)\s*=\s*register\(\s*\{")


def strip_comments(src: str) -> str:
    out = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if c in "\"'`":
            # string/template: copy verbatim (templates kept raw)
            j = i + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == c:
                    break
                j += 1
            out.append(src[i : j + 1])
            i = j + 1
        elif src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j == -1 else j
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            i = n if j == -1 else j + 2
        else:
            out.append(c)
            i += 1
    return "".join(out)


class Parser:
    """Lenient JS object-literal parser for declaration-shaped data."""

    def __init__(self, src: str):
        self.s = src
        self.i = 0
        self.n = len(src)

    def ws(self):
        while self.i < self.n and self.s[self.i] in " \t\r\n":
            self.i += 1

    def peek(self) -> str:
        self.ws()
        return self.s[self.i] if self.i < self.n else ""

    def value(self):
        self.ws()
        if self.i >= self.n:
            return None
        c = self.s[self.i]
        if c == "{":
            return self.obj()
        if c == "[":
            return self.arr()
        if c in "\"'":
            return self.string()
        if c == "`":
            raw = self.template()
            return {"$expr": raw} if "${" in raw else raw
        if c == "(" :
            return self.expr()
        # bare token: number / true / false / null / identifier / call / member
        return self.bare()

    def obj(self):
        result = {}
        self.i += 1  # {
        while True:
            before = self.i
            self.ws()
            if self.peek() == "}":
                self.i += 1
                return result
            if self.s.startswith("...", self.i):
                self.expr_until({",", "}"})
            else:
                key = self.key()
                self.ws()
                if self.peek() == ":":
                    self.i += 1
                    result[key] = self.value()
                elif self.peek() == "(":
                    # method shorthand — not descriptor data
                    self.skip_balanced()
                    if self.peek() == "{":
                        self.skip_balanced()
                else:
                    result[key] = {"$expr": key}
            self.ws()
            if self.peek() == ",":
                self.i += 1
            if self.i == before:
                # Progress guarantee: never spin on an unparsed shape.
                self.i += 1

    def arr(self):
        result = []
        self.i += 1  # [
        while True:
            self.ws()
            if self.peek() == "]":
                self.i += 1
                return result
            if self.s.startswith("...", self.i):
                result.append(self.expr_until({",", "]"}))
            else:
                result.append(self.value())
            self.ws()
            if self.peek() == ",":
                self.i += 1

    def key(self) -> str:
        self.ws()
        if self.peek() in "\"'":
            return self.string()
        match = re.match(r"[A-Za-z_$][\w$]*", self.s[self.i :])
        name = match.group(0) if match else ""
        self.i += len(name)
        return name

    def string(self) -> str:
        quote = self.s[self.i]
        self.i += 1
        out = []
        while self.i < self.n:
            c = self.s[self.i]
            if c == "\\" and self.i + 1 < self.n:
                nxt = self.s[self.i + 1]
                out.append({"n": "\n", "t": "\t", "r": "\r"}.get(nxt, nxt))
                self.i += 2
                continue
            if c == quote:
                self.i += 1
                break
            out.append(c)
            self.i += 1
        return "".join(out)

    def template(self) -> str:
        start = self.i
        self.i += 1
        while self.i < self.n:
            if self.s[self.i] == "\\":
                self.i += 2
                continue
            if self.s[self.i] == "`":
                self.i += 1
                break
            self.i += 1
        return self.s[start + 1 : self.i - 1]

    def skip_balanced(self):
        opener = self.s[self.i]
        closer = {"(": ")", "{": "}", "[": "]"}[opener]
        depth = 0
        while self.i < self.n:
            c = self.s[self.i]
            if c in "\"'`":
                if c == "`":
                    self.template()
                else:
                    self.string()
                continue
            if c == opener:
                depth += 1
            elif c == closer:
                depth -= 1
                if depth == 0:
                    self.i += 1
                    return
            self.i += 1

    def expr_until(self, stops: set) -> dict:
        start = self.i
        depth = 0
        while self.i < self.n:
            c = self.s[self.i]
            if c in "\"'`":
                if c == "`":
                    self.template()
                else:
                    self.string()
                continue
            if c in "{[(":
                depth += 1
            elif c in "}])":
                if depth == 0 and c in stops:
                    break
                depth -= 1
            elif depth == 0 and c in stops:
                break
            self.i += 1
        return {"$expr": self.s[start : self.i].strip()}

    def expr(self):
        return self.expr_until({",", "}", "]"})

    def bare(self):
        start = self.i
        rest = self.s[self.i :]
        match = re.match(r"-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?", rest)
        if match:
            self.i += len(match.group(0))
            num = match.group(0)
            return float(num) if any(ch in num for ch in ".eE") else int(num)
        match = re.match(r"[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*", rest)
        if match:
            name = match.group(0)
            self.i += len(name)
            if name == "true":
                return True
            if name == "false":
                return False
            if name in ("null", "undefined"):
                return None
            # Call, arrow function, or other compound expression: consume
            # the whole thing (from the identifier) as one opaque expr.
            save = self.i
            self.ws()
            if self.peek() == "(" or self.s.startswith("=>", self.i):
                self.i = start
                return self.expr()
            self.i = save
            return {"$expr": name}
        return self.expr()


CONST_DECL = re.compile(r"(?:export\s+)?const\s+(\w+)\s*=\s*(\[)")


def file_consts(src: str) -> dict:
    """Same-file `const NAME = [ ... ]` literal arrays, for resolving
    `values: SOME_CONST` references. Only flat literals resolve; anything
    else stays an opaque $expr."""
    consts = {}
    for match in CONST_DECL.finditer(src):
        start = match.end() - 1
        parser = Parser(src[start:])
        try:
            value = parser.value()
        except Exception:
            continue
        if isinstance(value, list) and all(isinstance(v, (str, int, float)) for v in value):
            consts[match.group(1)] = value
    return consts


def resolve(value, consts: dict):
    if isinstance(value, dict) and set(value) == {"$expr"} and value["$expr"] in consts:
        return consts[value["$expr"]]
    return value


def keep(descriptor: dict, consts: dict) -> dict | None:
    if not isinstance(descriptor, dict) or not isinstance(descriptor.get("id"), str):
        return None
    ident = descriptor["id"]
    out = {"id": ident, "type": descriptor.get("type")}
    values = resolve(descriptor.get("values"), consts)
    if isinstance(values, list):
        out["values"] = values
    if "default" in descriptor:
        out["default"] = resolve(descriptor["default"], consts)
    env = descriptor.get("env")
    if isinstance(env, str):
        out["env"] = env
    elif isinstance(env, dict) and isinstance(env.get("name"), str):
        out["env"] = env["name"]
    if descriptor.get("credential") is True:
        out["credential"] = True
    ui = descriptor.get("ui")
    if isinstance(ui, dict):
        kept = {}
        for field in ("tab", "group", "label", "description", "secret", "condition"):
            if field in ui:
                kept[field] = ui[field]
        options = ui.get("options")
        if isinstance(options, list):
            kept["options"] = [
                {
                    k: option[k]
                    for k in ("value", "label", "description")
                    if isinstance(option, dict) and k in option
                }
                for option in options
            ]
        elif options is not None:
            kept["options"] = options
        out["ui"] = kept
    return out


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    root = Path(sys.argv[1])
    out_path = Path(sys.argv[2])
    src_root = root / "packages" / "coding-agent" / "src"
    files = sorted(src_root.rglob("*.ts"))
    try:
        import subprocess

        revision = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip() or "unknown"
    except Exception:
        revision = "unknown"
    descriptors = []
    seen = set()
    for path in files:
        raw = path.read_text(encoding="utf-8", errors="replace")
        if "register({" not in raw:
            continue
        src = strip_comments(raw)
        consts = file_consts(src)
        for match in DECL.finditer(src):
            start = match.end() - 1  # at '{'
            end = None
            depth = 0
            i = start
            while i < len(src):
                c = src[i]
                if c in "\"'`":
                    quote = c
                    i += 1
                    while i < len(src):
                        if src[i] == "\\":
                            i += 2
                            continue
                        if src[i] == quote:
                            break
                        i += 1
                    i += 1
                    continue
                if c in "{[(":
                    depth += 1
                elif c in "}])":
                    depth -= 1
                    if depth == 0:
                        end = i + 1
                        break
                i += 1
            if end is None:
                continue
            literal = src[start:end]
            # drop a trailing `as const` inside arrays etc. is handled by
            # the parser's expression fallback; parse the object itself:
            parsed = Parser(literal).value()
            kept = keep(parsed, consts) if isinstance(parsed, dict) else None
            if kept and kept["id"] not in seen:
                seen.add(kept["id"])
                descriptors.append(kept)
    payload = {
        "source": "can1357/oh-my-pi",
        "revision": revision,
        "generatedBy": "scripts/gen-omp-settings.py",
        "settings": descriptors,
    }
    out_path.write_text(json.dumps(payload, indent=1, ensure_ascii=False) + "\n")
    expr_defaults = sum(
        1 for d in descriptors if isinstance(d.get("default"), dict) and "$expr" in d["default"]
    )
    print(f"{len(descriptors)} descriptors ({expr_defaults} computed defaults) -> {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
