//! Puts `assets/madar.ico` into the EXE as icon resource 1, without rc.exe.

use std::{env, fs, path::Path};

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
    let path = Path::new(&env::var("OUT_DIR").unwrap()).join("madar.res");
    fs::write(&path, res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", path.display());
}
