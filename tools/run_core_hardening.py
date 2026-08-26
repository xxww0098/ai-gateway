#!/usr/bin/env python3
from __future__ import annotations

import base64
import re
import zlib
from pathlib import Path

BOOTSTRAP = Path(__file__).with_name("one_shot_core_hardening.py")
text = BOOTSTRAP.read_text(encoding="utf-8")
payload = re.search(r'PAYLOAD = r"""\n(.*?)\n"""', text, re.S)
if payload is None:
    raise RuntimeError("embedded hardening payload not found")
code = zlib.decompress(
    base64.b85decode("".join(payload.group(1).split()))
).decode("utf-8")

old = '''replace_once(
    "crates/gw-provider/src/claude.rs",
    """        append_query(&mut parsed, query);
        Ok(parsed)
""",
    """        append_query(&mut parsed, raw_query, query);
        Ok(parsed)
""",
)
'''
new = '''path = "crates/gw-provider/src/claude.rs"
text = read(path)
old = """        append_query(&mut parsed, query);
        Ok(parsed)
"""
if old not in text:
    raise RuntimeError(f"{path}: messages endpoint append_query block not found")
write(
    path,
    text.replace(
        old,
        """        append_query(&mut parsed, raw_query, query);
        Ok(parsed)
""",
        1,
    ),
)
'''
if code.count(old) != 1:
    raise RuntimeError(f"expected one embedded Claude patch block, got {code.count(old)}")
code = code.replace(old, new, 1)

exec(compile(code, str(BOOTSTRAP), "exec"), {
    "__file__": str(BOOTSTRAP),
    "__name__": "__main__",
})
