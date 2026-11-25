//! Puts the app icon (resource 1), the manifest and the version (resource 16) into the EXE.

use std::{env, fs, path::Path};

/// Rounds `out` up to a 4-byte boundary.
fn pad(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(4), 0);
}

/// A VS_VERSIONINFO block: wLength, wValueLength, wType, UTF-16 key, padding,
/// value, children. wLength covers the block and its children, not the padding
/// after it, so the caller pads.
fn block(wtype: u16, name: &str, value: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0u8; 6];
    b.extend(name.encode_utf16().flat_map(u16::to_le_bytes));
    b.extend([0, 0]);
    pad(&mut b);
    b.extend(value);
    if !children.is_empty() {
        pad(&mut b);
        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                pad(&mut b);
            }
            b.extend(child);
        }
    }
    let len = (b.len() as u16).to_le_bytes();
    b[..2].copy_from_slice(&len);
    b[2..4].copy_from_slice(&(value.len() as u16).to_le_bytes());
    b[4..6].copy_from_slice(&wtype.to_le_bytes());
    b
}

/// A NUL-terminated UTF-16 string as bytes.
fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).chain([0, 0]).collect()
}

/// The VS_VERSIONINFO resource, its version read from Cargo.toml.
fn version() -> Vec<u8> {
    let part = |name: &str| -> u32 {
        env::var(name)
            .unwrap_or_else(|_| panic!("{name} is missing"))
            .parse()
            .unwrap_or_else(|_| panic!("{name} is not a number"))
    };
    let (major, minor, patch) = (
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH"),
    );
    let text = format!("{major}.{minor}.{patch}");
    let (ms, ls) = ((major << 16) | minor, patch << 16);
    let mut fixed = Vec::with_capacity(52);
    for v in [
        0xFEEF04BD, 0x0001_0000, ms, ls, ms, ls, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0,
    ] {
        fixed.extend(v.to_le_bytes());
    }
    let strings = [
        ("CompanyName", "Ali Alhumedi"),
        ("FileDescription", "Madar"),
        ("FileVersion", text.as_str()),
        ("InternalName", "madar"),
        ("LegalCopyright", "© 2026 Ali Alhumedi"),
        ("OriginalFilename", "madar.exe"),
        ("ProductName", "Madar"),
        ("ProductVersion", text.as_str()),
    ]
    .map(|(key, value)| block(1, key, &wide(value), &[]));
    let table = block(1, "040904B0", &[], &strings);
    let string_info = block(1, "StringFileInfo", &[], &[table]);
    // Translation: language 0x0409, code page 0x04B0, as two u16 values.
    let translation = block(0, "Translation", &[0x09, 0x04, 0xB0, 0x04], &[]);
    let var_info = block(1, "VarFileInfo", &[], &[translation]);
    block(0, "VS_VERSION_INFO", &fixed, &[string_info, var_info])
}

fn resource(out: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    for v in [0xFFFF, kind, 0xFFFF, id] {
        out.extend(v.to_le_bytes());
    }
    out.extend(0u32.to_le_bytes());
    // Memory flags, then US English.
    out.extend(0x1010u16.to_le_bytes());
    out.extend(0x0409u16.to_le_bytes());
    out.extend([0; 8]);
    out.extend(data);
    out.resize(out.len().next_multiple_of(4), 0);
}

fn main() {
    println!("cargo:rerun-if-changed=assets/madar.ico");
    println!("cargo:rerun-if-changed=assets/madar.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    let ico = fs::read("assets/madar.ico").expect("assets/madar.ico");
    let mut res = Vec::new();
    // Every .res file starts with an empty resource.
    resource(&mut res, 0, 0, &[]);
    res[20..24].fill(0);
    let mut group = ico[..6].to_vec();
    for (i, entry) in ico[6..].chunks(16).take(ico[4] as usize).enumerate() {
        let at = |o: usize| u32::from_le_bytes(entry[o..o + 4].try_into().unwrap()) as usize;
        let id = i as u16 + 1;
        resource(&mut res, 3, id, &ico[at(12)..][..at(8)]);
        group.extend(&entry[..12]);
        group.extend(id.to_le_bytes());
    }
    resource(&mut res, 14, 1, &group);
    let manifest = fs::read("assets/madar.manifest").expect("assets/madar.manifest");
    resource(&mut res, 24, 1, &manifest);
    resource(&mut res, 16, 1, &version());
    let path = Path::new(&env::var("OUT_DIR").unwrap()).join("madar.res");
    fs::write(&path, res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", path.display());
}
