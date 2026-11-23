import os
import sys
import tomllib

RS = ".rs"
PY = ".py"
TOML = ".toml"


def skip_dq(src, i):
    n = len(src)
    j = i + 1
    while j < n:
        c = src[j]
        if c == "\\":
            j += 2
            continue
        if c == '"':
            return j + 1
        j += 1
    return n


def skip_raw(src, i, hashes):
    n = len(src)
    j = i + 1
    tail = "#" * hashes
    while j < n:
        if src[j] == '"':
            k = j + 1
            while k < n and src[k] == "#" and k - j - 1 < hashes:
                k += 1
            if k - j - 1 == hashes:
                return k
            j += 1
            continue
        j += 1
    return n


def skip_char(src, i):
    n = len(src)
    j = i + 1
    if j >= n:
        return n
    if src[j] == "\\":
        j += 1
        if j < n and src[j] == "u":
            j += 1
            if j < n and src[j] == "{":
                while j < n and src[j] != "}":
                    j += 1
                j += 1
        elif j < n and src[j] == "x":
            j += 3
        else:
            j += 1
        if j < n and src[j] == "'":
            return j + 1
        return min(j + 1, n)
    if j + 1 < n and src[j + 1] == "'":
        return j + 2
    while j < n and (src[j].isalnum() or src[j] == "_"):
        j += 1
    return j


def rust_spans(src):
    spans = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = i + 2
            while j < n and src[j] != "\n":
                j += 1
            spans.append((i, j))
            i = j
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            depth = 1
            j = i + 2
            while j < n and depth > 0:
                if j + 1 < n and src[j] == "/" and src[j + 1] == "*":
                    depth += 1
                    j += 2
                elif j + 1 < n and src[j] == "*" and src[j + 1] == "/":
                    depth -= 1
                    j += 2
                else:
                    j += 1
            spans.append((i, j))
            i = j
            continue
        if c == "'":
            i = skip_char(src, i)
            continue
        boundary = i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")
        if c == '"' and boundary:
            i = skip_dq(src, i)
            continue
        if boundary and c in "rbc":
            j = i + 1
            if c == "r":
                hashes = 0
                while j < n and src[j] == "#":
                    hashes += 1
                    j += 1
                if j < n and src[j] == '"':
                    i = skip_raw(src, j, hashes)
                    continue
            elif c == "b":
                if j < n and src[j] == '"':
                    i = skip_dq(src, j)
                    continue
                if j < n and src[j] == "'":
                    i = skip_char(src, j)
                    continue
                if j < n and src[j] == "r":
                    j2 = j + 1
                    hashes = 0
                    while j2 < n and src[j2] == "#":
                        hashes += 1
                        j2 += 1
                    if j2 < n and src[j2] == '"':
                        i = skip_raw(src, j2, hashes)
                        continue
            elif c == "c":
                if j < n and src[j] == '"':
                    i = skip_dq(src, j)
                    continue
                if j < n and src[j] == "r":
                    j2 = j + 1
                    hashes = 0
                    while j2 < n and src[j2] == "#":
                        hashes += 1
                        j2 += 1
                    if j2 < n and src[j2] == '"':
                        i = skip_raw(src, j2, hashes)
                        continue
        i += 1
    return spans


def python_spans(src):
    spans = []
    i, n = 0, len(src)
    if src.startswith("#!"):
        nl = src.find("\n")
        i = n if nl < 0 else nl + 1
    while i < n:
        c = src[i]
        if c == "#":
            j = i
            while j < n and src[j] != "\n":
                j += 1
            spans.append((i, j))
            i = j
            continue
        if c in "\"'":
            if src.startswith(c * 3, i):
                q = c * 3
                j = i + 3
                while j < n and not src.startswith(q, j):
                    if src[j] == "\\":
                        j += 2
                    else:
                        j += 1
                i = min(j + 3, n)
                continue
            j = i + 1
            while j < n and src[j] != c:
                if src[j] == "\\":
                    j += 2
                else:
                    j += 1
            i = min(j + 1, n)
            continue
        i += 1
    return spans


def toml_spans(src):
    spans = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if c == "#":
            j = i
            while j < n and src[j] != "\n":
                j += 1
            spans.append((i, j))
            i = j
            continue
        if c in "\"'":
            if src.startswith(c * 3, i):
                q = c * 3
                j = i + 3
                while j < n and not src.startswith(q, j):
                    j += 1
                i = min(j + 3, n)
                continue
            j = i + 1
            while j < n and src[j] != c:
                if c == '"' and src[j] == "\\":
                    j += 2
                else:
                    j += 1
            i = min(j + 1, n)
            continue
        i += 1
    return spans


def spans_for(path, src):
    ext = os.path.splitext(path)[1]
    if ext == RS:
        return rust_spans(src)
    if ext == PY:
        return python_spans(src)
    if ext == TOML:
        return toml_spans(src)
    return []


def apply_spans(src, spans):
    if not spans:
        return src
    lines = src.split("\n")
    starts = []
    pos = 0
    for ln in lines:
        starts.append(pos)
        pos += len(ln) + 1
    out = []
    for k, ln in enumerate(lines):
        ls = starts[k]
        le = ls + len(ln)
        inside = [False] * len(ln)
        for s, e in spans:
            if e <= ls or s >= le:
                continue
            for t in range(max(s, ls) - ls, min(e, le) - ls):
                inside[t] = True
        if len(ln) > 0 and all(inside[t] or ln[t].isspace() for t in range(len(ln))):
            continue
        chars = []
        t = 0
        while t < len(ln):
            if inside[t]:
                chars.append(" ")
                while t < len(ln) and inside[t]:
                    t += 1
            else:
                chars.append(ln[t])
                t += 1
        new = "".join(chars)
        if any(inside):
            new = new.rstrip()
        out.append(new)
    return "\n".join(out)


def iter_targets(root):
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in (".git", "target")]
        for name in filenames:
            if os.path.splitext(name)[1] in (RS, PY, TOML):
                yield os.path.join(dirpath, name)


def main():
    root = os.environ.get("APORIA_ROOT")
    if not root:
        root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    mode = sys.argv[1] if len(sys.argv) > 1 else "check"
    changed = 0
    dirty = []
    for path in iter_targets(root):
        with open(path, "r", encoding="utf-8") as fh:
            src = fh.read()
        spans = spans_for(path, src)
        if not spans:
            continue
        if mode == "check":
            dirty.append((os.path.relpath(path, root), len(spans)))
            continue
        stripped = apply_spans(src, spans)
        if stripped != src:
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(stripped)
            changed += 1
    if mode == "check":
        if dirty:
            for name, count in dirty:
                print(f"{name}: {count} comment(s)")
            print(f"FAIL: {len(dirty)} file(s) contain comments")
            sys.exit(1)
        print("OK: no comments in code files")
    else:
        print(f"stripped comments in {changed} file(s)")


if __name__ == "__main__":
    main()
