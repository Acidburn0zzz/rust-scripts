#![feature(path_ext, core)]
#![feature(test)]

extern crate nix;

#[cfg(test)]
extern crate test;

use std::env;
use std::io::{self, Write};
use std::fmt;
use std::path::Path;
use std::fs::PathExt;

use nix::sys::stat::stat;

#[cfg(test)]
use test::Bencher;

fn automount_name() -> String {
    env::var("ID_FS_LABEL").or_else(|_| env::var("ID_FS_UUID")).unwrap_or_else(|_| {
        format!("{}_{}_{}", env::var("ID_VENDOR").unwrap(), env::var("ID_MODEL").unwrap(), env::args().nth(1).unwrap())
    })
}

fn ismount(dir: &str) -> bool {
    let path = Path::new(dir);

    path.is_dir() && {
        (match stat(match path.parent() {
            Some(p) if p == Path::new("/") => { println!("parent {:?}",p); Path::new("")},
            Some(p) => p,
            None => return true
        }) {
            Ok(s) => { println!("parent stat {:?}", s.st_dev); s },
            Err(e) => return { println!("parent stat err {:?} {:?}", path.parent(), e); false }
        }).st_dev != (match stat(path) {
            Ok(s) => {println!("my stat {:?}", s.st_dev); s },
            Err(e) => return { println!("my stat err {:?} {:?}", path, e); false }
        }).st_dev
    }

}

fn systemd_encode(inp: &str) -> String {
    let mut out = String::new();
    for &b in inp.as_bytes().iter() {
        if ('a' as u8) <= b && b <= ('z' as u8)
            || ('A' as u8) <= b && b <= ('Z' as u8)
            || ('0' as u8) <= b && b <= ('9' as u8)
            || b == ('_' as u8) { unsafe{ out.as_mut_vec().push(b); } }
        else {
            out.push_str(r"\x");
            out.push_str(&*fmt::radix(b, 16).to_string());
        }
    }
    out
}

fn main() {
    let mut name = automount_name();

    while ismount(&*format!("/media/{}", name)) {
        name = name + "_";
    }

    let service_name = format!("{} /media/{}", env::var("DEVNAME").unwrap(), name);

    let mut out = io::stdout();
    out.write_all(name.as_bytes()).unwrap();
    out.write(&[0x10]).unwrap();
    out.write_all(systemd_encode(&*service_name).as_bytes()).unwrap();
    out.write(&[0x10]).unwrap();
    out.flush().unwrap();
}

#[test]
fn test_ismount() {
    assert_eq!(ismount("/"), true);
    assert_eq!(ismount("/tmp"), true);
    assert_eq!(ismount("/non-existant"), false);
    assert_eq!(ismount("/usr/bin"), false);
}

#[test]
fn test_systemd_encode() {
    assert_eq!(&*systemd_encode("hello_W0rld"), "hello_W0rld");
    assert_eq!(&*systemd_encode(r"/dev/sda1 /media/path"), r"\x2fdev\x2fsda1\x20\x2fmedia\x2fpath");
}

#[test]
fn test_automount_name() {
    env::remove_var("ID_FS_UUID");
    env::remove_var("ID_FS_LABEL");

    // TODO: how to fake os::args()?
    //env::set_var("ID_VENDOR", "Vendor");
    //env::set_var("ID_MODEL", "Model");
    //assert_eq!(&*automount_name(), "Vendor_Model_1");

    env::set_var("ID_FS_UUID", "UUID");
    assert_eq!(&*automount_name(), "UUID");

    env::set_var("ID_FS_LABEL", "LABEL");
    assert_eq!(&*automount_name(), "LABEL");
}

#[bench]
fn bench_systemd_encode(b: &mut Bencher) {
    b.iter(|| {
        systemd_encode(r"/dev/sda1 /media/path")
    });
}

#[bench]
fn bench_ismount(b: &mut Bencher) {
    b.iter(|| {
        ismount("/tmp")
    });
}

#[bench]
fn bench_automount_name_label(b: &mut Bencher) {
    env::set_var("ID_FS_LABEL", "LABEL");
    env::remove_var("ID_FS_UUID");
    b.iter(|| {
        automount_name()
    });
}

#[bench]
fn bench_automount_name_uuid(b: &mut Bencher) {
    env::set_var("ID_FS_UUID", "UUID");
    env::remove_var("ID_FS_LABEL");
    b.iter(|| {
        automount_name()
    });
}
