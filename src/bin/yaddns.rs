#![feature(custom_derive, plugin)]
#![feature(custom_attribute)]
#![plugin(serde_macros)]

// TODO
#![allow(dead_code, unused_variables)]

extern crate pb;
extern crate script_utils as utils;
extern crate yadns;
extern crate serde;

use std::env;
use std::net::Ipv4Addr;
use yadns::{YandexDNS, ListRequest, AddRequest, DnsType};

#[derive(Debug, Clone, Deserialize)]
struct Config {
    domain: String,
    token: String
}

#[derive(Debug, Clone, Deserialize)]
struct PbConfig {
    access_token: String,
    device_iden: Option<String>
}

fn get_my_ip_address() -> Option<Ipv4Addr> {
    use std::net::{TcpStream, SocketAddr};
    let addr = TcpStream::connect("8.8.8.8:53").and_then(|s| s.local_addr());
    match addr {
        Ok(SocketAddr::V4(addr)) => Some(*addr.ip()),
        _ => None,
    }
}

fn main() {
    let pbcfg = utils::load_config::<PbConfig>("pushbullet/config.toml").unwrap();
    let config = utils::load_config::<Config>("yadns/config.toml").unwrap();

    let my_ip_addr = env::args().nth(4).or_else(|| get_my_ip_address().map(|v| v.to_string()));

    let mut yadns = YandexDNS::new(&*config.token);
    let home_record = yadns.send(&ListRequest::new(&*config.domain))
        .unwrap().records.into_iter()
        .find(|rec| rec.kind == DnsType::A && rec.subdomain == "home");

    match home_record {
        Some(rec) => {
            yadns.send(
                rec.as_edit_req()
                .content(&*my_ip_addr.unwrap()))
                .unwrap();
        },
        None => {
            yadns.send(
                AddRequest::new(DnsType::A, &*config.domain)
                .subdomain("home")
                .content("127.0.0.1"))
                .unwrap();
        },
    }
}

