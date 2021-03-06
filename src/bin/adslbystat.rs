#![cfg_attr(test, feature(test))]

#[cfg(test)]
extern crate test;
extern crate encoding;
extern crate toml;
extern crate hyper;
extern crate url;
extern crate serde;
extern crate regex;
extern crate script_utils as utils;

use regex::Regex;
use hyper::client::Client;
use hyper::header::{Authorization, Referer, Basic};
use hyper::error::Result;
use encoding::{Encoding, DecoderTrap};
use encoding::all::WINDOWS_1251;
use std::process::exit;
use std::env;
use std::fmt;
use std::io::Read;

#[cfg(test)]
use test::Bencher;

#[derive(Debug)]
struct AcctInfo {
    enabled: bool,
    account: f32,
    days: i32,
    price: i32,
    credit: Option<i32>,
}

impl fmt::Display for AcctInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        try!(writeln!(f, "Enabled: {}", self.enabled));
        try!(writeln!(f, "Account: {:.2} rub", self.account));
        try!(writeln!(f, "Days left: {}", self.days));
        try!(writeln!(f, "Price per Mib: {} rub", self.price));
        if let Some(ref c) = self.credit {
            try!(writeln!(f, "Allowed credit: {}%", c));
        }
        Ok(())
    }
}

include!(concat!(env!("OUT_DIR"), "/adslbystat.rs"));

fn enable_credit(creds: Creds) -> Result<bool> {
    let client = Client::new();
    client.get("https://www.adsl.by/credit.js?credit=on")
          .header(Authorization(Basic {
              username: creds.username,
              password: Some(creds.password),
          }))
          .header(Referer("https://www.adsl.by".into()))
          .send()
          .and_then(|mut resp| {
              let mut buf = String::new();
              resp.read_to_string(&mut buf)
                  .map_err(::hyper::error::Error::Io)
                  .map(|_| buf.contains("stat: 'Включен'"))
          })
}

const EXIT_ENABLED: i32 = 0;
const EXIT_DISABLED: i32 = 1;
const EXIT_ERROR: i32 = 2;

fn main() {
    let state_re = Regex::new(r">Аккаунт</td>\s*<td class='right'><b>Включен<").unwrap();
    let account_re = Regex::new(r"Осталось трафика на сумму</td>\s*<td class='right'><b>(-?[0-9. ]+)").unwrap();
    let days_re = Regex::new(r"осталось <b>(-?\d+) д").unwrap();
    let price_re = Regex::new(r"тариф</td>\s*<td class='right'><b>(\d+) ").unwrap();
    let credit_re = Regex::new(r"кредит</td>\s*<td class='right'><b>(\d+)%").unwrap();

    let config: Creds = match utils::load_config("adslby/creds.toml") {
        Some(conf) => conf,
        None => {
            println!("Config file load error.");
            exit(EXIT_ERROR);
        }
    };

    let client = Client::new();
    // client.set_ssl_verifier(Box::new(utils::permissive_ssl_checker));

    let buf = match client.get("https://www.adsl.by/001.htm")
                          .header(Authorization(Basic {
                              username: config.username.clone(),
                              password: Some(config.password.clone()),
                          }))
                          .send()
                          .and_then(|mut resp| {
                              let mut buf = Vec::new();
                              resp.read_to_end(&mut buf)
                                  .map(|_| buf)
                                  .map_err(::hyper::error::Error::Io)
                          }) {
        Ok(buf) => buf,
        Err(err) => {
            println!("Error requesting account stats: {}", err);
            exit(EXIT_ERROR);
        }
    };

    let cont = match WINDOWS_1251.decode(&*buf, DecoderTrap::Replace) {
        Ok(res) => res,
        Err(err) => {
            println!("Error decoding HTML page: {}", err);
            exit(EXIT_ERROR);
        }
    };

    let acct = AcctInfo {
        enabled: state_re.is_match(&*cont),
        account: account_re.captures(&*cont)
                           .and_then(|c| c.at(1).and_then(|v| v.replace(" ", "").parse().ok()))
                           .unwrap_or(0f32),
        days: days_re.captures(&*cont)
                     .and_then(|c| c.at(1).and_then(|v| v.parse().ok()))
                     .unwrap_or(0),
        price: price_re.captures(&*cont)
                       .and_then(|c| c.at(1).and_then(|v| v.parse().ok()))
                       .unwrap_or(0),
        credit: credit_re.captures(&*cont).and_then(|c| c.at(1).and_then(|v| v.parse().ok())),
    };

    println!("{}", acct);

    exit(if acct.enabled {
        EXIT_ENABLED
    } else {
        if env::args().position(|arg| arg == "credit").is_some() {
            match enable_credit(config) {
                Ok(true) => {
                    println!("Credit was enabled.");
                    EXIT_ENABLED
                }
                Ok(false) => {
                    println!("Credit was not enabled.");
                    EXIT_DISABLED
                }
                Err(err) => {
                    println!("Error enabling credit: {}", err);
                    EXIT_ERROR
                }
            }
        } else {
            EXIT_DISABLED
        }
    });
}

#[bench]
#[ignore]
fn bench_main(b: &mut Bencher) {
    b.iter(|| main());
}
