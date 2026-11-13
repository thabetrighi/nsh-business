#!/usr/bin/env python3
"""Operation log for replicas (M11/M0, ADR-0006).

Every writing use case in crates/nb-store/src/usecases records its call
(name and arguments) before it writes, so the operation can be run again
on another replica (the server) and make the same rows. This script:

1. finds each `pub fn name(store: &mut Store, meta: OpMeta, args…) -> …`;
2. makes sure its body starts with `store.record(meta, …)` (adds it);
3. writes crates/nb-store/src/ops.rs: the `Call` enum and `replay`.

Run it after adding or changing a writing use case:
    python3 tools/gen_ops.py && cargo fmt --all
"""
import re, glob, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "crates/nb-store/src")

# Arguments that must never be written to the log (passwords, PINs): the
# operation is logged as opaque and synced by its rows (see SYNC_PLAN §5).
SENSITIVE = {"Secret", "UserInput", "UserUpdate"}
# Device-local operations: never replayed elsewhere.
LOCAL_TYPES = {"&Path"}
LOCAL_FNS = {"backups::"}

SIG = re.compile(
    r"pub fn (\w+)\(\s*store: &mut Store,\s*meta: OpMeta,?(.*?)\)\s*->\s*([^{]+?)\s*\{",
    re.S,
)

def split_args(s):
    out, depth, cur = [], 0, ""
    for ch in s:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    out.append(cur)
    return [a.strip() for a in out if a.strip()]

# Use cases whose result is not serializable: replay gives null.
DISCARD_RESULT = {"org::set_feature"}

def imports(text):
    """Type names a module imports, with their full path."""
    out = {}
    for m in re.finditer(r"^use ([\w:]+?)::(?:\{([^}]*)\}|(\w+));", text, re.M):
        base = m.group(1)
        names = m.group(2).split(",") if m.group(2) else [m.group(3)]
        for n in names:
            n = n.strip()
            if not n or n == "self":
                continue
            if " as " in n:
                orig, alias = [x.strip() for x in n.split(" as ")]
            else:
                orig = alias = n
            if alias[:1].isupper():
                out[alias] = f"{base}::{orig}"
    return out

def qualify(t, module, defined, imported):
    """Writes the module's types with their full path."""
    def full(m):
        n = m.group(1)
        if n in defined:
            return f"crate::usecases::{module}::{n}"
        return imported.get(n, n)
    return re.sub(r"(?<![:\w])([A-Z]\w*)\b", full, t)

def owned(t):
    """(stored type, expression to store `x`, expression to pass back `x`)."""
    t = t.strip()
    if t == "&str":
        return "String", "{x}.to_owned()", "&{x}"
    if t == "Option<&str>":
        return "Option<String>", "{x}.map(str::to_owned)", "{x}.as_deref()"
    m = re.fullmatch(r"&\[(.+)\]", t)
    if m:
        return f"Vec<{m.group(1)}>", "{x}.to_vec()", "&{x}"
    if t.startswith("&"):
        return t[1:], "{x}.clone()", "&{x}"
    return t, "{x}.clone()", "{x}"

def rustfmt(text):
    """The generated module as `cargo fmt` would leave it."""
    import subprocess
    cfg = os.path.join(ROOT, "rustfmt.toml")
    args = ["rustfmt", "--edition", "2024", "--emit", "stdout"]
    if os.path.exists(cfg):
        args += ["--config-path", cfg]
    r = subprocess.run(args, input=text, capture_output=True, text=True)
    return r.stdout if r.returncode == 0 and r.stdout.strip() else text

def squash(s):
    """Text without its layout (rustfmt may wrap the generated lines)."""
    return re.sub(r"\s+", "", s).replace(",)", ")").replace(",}", "}").replace(",]", "]")

def camel(s):
    return "".join(p[:1].upper() + p[1:] for p in s.split("_") if p)

def main(check=False):
    variants, arms, changed = [], [], []
    for path in sorted(glob.glob(os.path.join(SRC, "usecases/*.rs"))):
        module = os.path.basename(path)[:-3]
        text = open(path).read()
        defined = set(re.findall(r"pub (?:struct|enum|type) (\w+)", text))
        imported = imports(text)
        out, pos = [], 0
        for m in SIG.finditer(text):
            name, args, ret = m.group(1), m.group(2), m.group(3)
            params = []
            for a in split_args(args):
                n, t = a.split(":", 1)
                n = n.strip()
                n = n[4:].strip() if n.startswith("mut ") else n
                params.append((n, qualify(t.strip(), module, defined, imported)))
            full = f"{module}::{name}"
            types = {t for _, t in params}
            local = any(full.startswith(p) for p in LOCAL_FNS) or types & LOCAL_TYPES
            if local:
                continue
            variant = camel(module) + camel(name)
            opaque = any(any(s in t for s in SENSITIVE) for t in types)
            if opaque:
                record = f'store.record(meta, &crate::ops::Call::Opaque {{ name: "{full}".into() }})?;'
            else:
                fields = []
                for n, t in params:
                    st, keep, _ = owned(t)
                    fields.append(f"{n}: {keep.format(x=n)}")
                record = (
                    f"store.record(meta, &crate::ops::Call::{variant} {{ {', '.join(fields)} }})?;"
                    if fields
                    else f"store.record(meta, &crate::ops::Call::{variant})?;"
                )
                vfields = ", ".join(f"{n}: {owned(t)[0]}" for n, t in params)
                variants.append(f"    {variant} {{ {vfields} }}," if params else f"    {variant},")
                pat = ", ".join(n for n, _ in params)
                call_args = ", ".join(["store", "meta"] + [owned(t)[2].format(x=n) for n, t in params])
                head = f"Call::{variant} {{ {pat} }}" if params else f"Call::{variant}"
                if full in DISCARD_RESULT:
                    arms.append(
                        f"        {head} => {{ crate::usecases::{module}::{name}({call_args})?; serde_json::Value::Null }}"
                    )
                else:
                    arms.append(
                        f"        {head} => serde_json::to_value(crate::usecases::{module}::{name}({call_args})?)?,"
                    )
            # The body must start with the record line (replace an old one).
            body_start = m.end()
            out.append(text[pos:body_start])
            rest = text[body_start:]
            old = re.match(r"\s*(?:#\[allow\(clippy::clone_on_copy\)\]\s*)?store\.record\(\s*meta,\s*&crate::ops::Call::[^;]*;", rest, re.S)
            if old:
                rest_from = body_start + old.end()
            else:
                rest_from = body_start
            out.append("\n    #[allow(clippy::clone_on_copy)]\n    " + record)
            pos = rest_from
        out.append(text[pos:])
        new = "".join(out)
        if squash(new) != squash(text):
            changed.append(path)
            if not check:
                open(path, "w").write(new)
    ops = OPS_TEMPLATE.replace("{variants}", "\n".join(variants)).replace("{arms}", "\n".join(arms))
    ops = rustfmt(ops)
    ops_path = os.path.join(SRC, "ops.rs")
    if not os.path.exists(ops_path) or squash(open(ops_path).read()) != squash(ops):
        changed.append(ops_path)
        if not check:
            open(ops_path, "w").write(ops)
    return changed

OPS_TEMPLATE = '''//! The operation log (M11/M0, ADR-0006). GENERATED by `tools/gen_ops.py`
//! from the writing use cases: do not edit by hand.
//!
//! Each writing use case records its call before it writes; the call is
//! kept with the operation (`operations.call`). [`replay`] runs a recorded
//! call again, on another replica, as the same operation (same id, same
//! time, same device): the rows it makes are the same.

#![allow(clippy::too_many_lines, clippy::large_enum_variant)]

use nb_domain::*;
use serde::{Deserialize, Serialize};

use crate::{OpMeta, Result, Store};

/// A writing use case and its arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "fn", content = "args")]
pub enum Call {
    /// Its arguments hold secrets (passwords, PINs): not kept; the
    /// operation travels as its rows.
    Opaque { name: String },
{variants}
}

/// An operation as kept in the log (and sent to the server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggedOp {
    pub id: nb_core::OperationId,
    pub kind: String,
    pub org: Option<nb_core::OrganizationId>,
    pub actor: Option<UserId>,
    /// When it ran (its rows' time).
    pub at: i64,
    /// The device that ran it (its rows carry it, its numbers its prefix);
    /// `None`: this replica's own device.
    #[serde(default)]
    pub device: Option<OpDevice>,
    pub call: Call,
}

/// The device an operation ran on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpDevice {
    pub id: nb_core::DeviceId,
    pub prefix: String,
}

/// The name of an operation kind as the `'static` text [`OpMeta`] holds
/// (a replica sees the same few kinds again and again).
pub fn intern(kind: &str) -> &'static str {
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    static KINDS: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let Ok(mut set) = KINDS.lock() else {
        return "replay";
    };
    if let Some(k) = set.get(kind) {
        return k;
    }
    let k: &'static str = Box::leak(kind.to_owned().into_boxed_str());
    set.insert(k);
    k
}

/// Runs a recorded call again as operation `meta`.
pub fn replay(store: &mut Store, meta: OpMeta, call: Call) -> Result<serde_json::Value> {
    Ok(match call {
        Call::Opaque { name } => return Err(crate::Error::Corrupt(format!("opaque operation {name}"))),
{arms}
    })
}
'''

if __name__ == "__main__":
    check = "--check" in sys.argv
    changed = main(check)
    if check and changed:
        print("out of date:", *changed, sep="\n  ")
        sys.exit(1)
    print("ok" if not changed else f"updated {len(changed)} files")
