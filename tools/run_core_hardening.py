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

marker = 'print("core hardening patch applied")'
extra = r'''
# Keep the non-billable count-tokens route on the same exact-query and usage
# handle contracts as the inference routes.
replace_once(
    "crates/gw-proxy/src/routes.rs",
    """pub(crate) use routing::{
    RoutedProvider, dialect_error, partition_routable, rewrite_model, select_upstreams,
};
""",
    """pub(crate) use routing::{
    dialect_error, partition_routable, rewrite_model, select_upstreams,
};
""",
)
replace_once(
    "crates/gw-proxy/src/routes/catalogue.rs",
    """use super::stream::{Relayed, relay_response, usage_probe};
""",
    """use super::stream::{Relayed, relay_response};
use super::translation::usage_probe;
""",
)
replace_once(
    "crates/gw-proxy/src/routes/catalogue.rs",
    """            headers: headers.clone(),
            query: query.clone(),
""",
    """            headers: headers.clone(),
            query: Vec::new(),
            raw_query: Some(query.clone()),
""",
)
replace_once(
    "crates/gw-proxy/src/routes/catalogue.rs",
    """        return match state.dispatch.send(&plan, &request, outgoing, probe).await {
""",
    """        return match state
            .dispatch
            .send(&plan, &request, outgoing, Some(probe))
            .await
        {
""",
)
'''
if code.count(marker) != 1:
    raise RuntimeError("hardening script completion marker is missing")
code = code.replace(marker, extra + "\n" + marker, 1)

exec(compile(code, str(BOOTSTRAP), "exec"), {
    "__file__": str(BOOTSTRAP),
    "__name__": "__main__",
})


def replace_generated(path: str, old: str, new: str) -> None:
    target = Path(path)
    generated = target.read_text(encoding="utf-8")
    if generated.count(old) != 1:
        raise RuntimeError(
            f"{path}: expected one generated occurrence of {old!r}, "
            f"got {generated.count(old)}"
        )
    target.write_text(generated.replace(old, new, 1), encoding="utf-8")


# Clippy is a release gate. `finish()` returns an Option, not a guard; binding
# it to `_` communicates intentional discard without invoking drop_non_drop.
replace_generated(
    "crates/gw-relay/src/probe/tests.rs",
    "    drop(probe.finish());\n",
    "    let _ = probe.finish();\n",
)

# The enum variant is visible to the whole routes module, so its payload type
# must be visible at the same boundary. This stays crate-internal.
replace_generated(
    "crates/gw-proxy/src/routes/translation.rs",
    "struct SharedUsage(Arc<Mutex<Option<Option<RelayUsage>>>>);",
    "pub(super) struct SharedUsage(Arc<Mutex<Option<Option<RelayUsage>>>>);",
)
