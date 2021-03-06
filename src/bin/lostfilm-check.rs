extern crate encoding;
extern crate hyper;
extern crate cookie;
extern crate url;
extern crate regex;
extern crate serde;
extern crate script_utils as utils;
extern crate xml;
extern crate pb;
#[macro_use]
extern crate log;

use std::io::Read;

use encoding::{Encoding, DecoderTrap};
use encoding::all::WINDOWS_1251;

use hyper::client::{Client, RedirectPolicy};
use hyper::status::StatusCode;
use hyper::error::Error as HttpError;
use hyper::header::{ContentType, UserAgent, Cookie, SetCookie, Referer};
use hyper::header::{Header, HeaderFormat};

use cookie::CookieJar;
use cookie::Cookie as CookiePair;

use url::{Url, UrlParser, form_urlencoded};
use xml::reader::{EventReader, XmlEvent};
use xml::name::OwnedName;
use regex::Regex;
use pb::{PbAPI, PushMsg, TargetIden, Push, PushData};

static USER_AGENT: &'static str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, \
                                   like Gecko) Chrome/33.0.1750.152 Safari/537.36";
static TRANSMISSION_URL: &'static str = "http://localhost:9091/transmission/rpc";
static BASE_URL: &'static str = "http://www.lostfilm.tv/";
static LOGIN_URL: &'static str = "http://login1.bogi.ru/login.php";

include!(concat!(env!("OUT_DIR"), "/lostfilm-check.rs"));

fn notify(api: &mut PbAPI, device_iden: Option<String>, title: &str, url: &str) {
    println!("added torrent {}: {}", title, url);

    let push = PushMsg {
        title: Some("New LostFilm release".into()),
        body: Some(title.into()),
        target: TargetIden::CurrentUser,
        data: PushData::Link(Url::parse(url).ok()),
        source_device_iden: device_iden,
    };

    if let Ok(result @ Push {..}) = api.send(&push) {
        println!("notified with push {}", result.iden);
    }
}

macro_rules! qs {
    ($($key:expr => $value:expr),*) => {
        vec![$(($key, $value)),*]
    }
}

#[allow(unused_must_use)]
fn login<'a>(login: &str, password: &str) -> CookieJar<'a> {
    debug!("trying to login as {}...", login);
    let base_url = Url::parse(BASE_URL).unwrap();
    let mut parser = UrlParser::new();
    parser.base_url(&base_url);

    let mut url = parser.parse(LOGIN_URL).unwrap();
    url.set_query_from_pairs(vec![("referer", BASE_URL)].into_iter());

    let data = form_urlencoded::serialize(qs![
        "login" => login,
        "password" => password,
        "module" => "1",
        "target" => BASE_URL,
        "repage" => "user",
        "act" => "login"
    ].into_iter());

    let input_re = Regex::new("<input .*?name=\"(\\w+)\" .*?value=\"([^\"]*)\"").unwrap();
    let action_re = Regex::new("action=\"([^\"]+)\"").unwrap();

    let mut cookie_jar = CookieJar::new(b"3b53fc89707a78fae45eeafff931f054");

    let mut client = Client::new();

    debug!("running first stage...");
    client.set_redirect_policy(RedirectPolicy::FollowAll);
    let mut response = client.post(url)
                             .body(&*data)
                             .header(ContentType("application/x-www-form-urlencoded"
                                                     .parse()
                                                     .unwrap()))
                             .header(UserAgent(USER_AGENT.to_string()))
                             .header(Referer(BASE_URL.to_string()))
                             .send()
                             .unwrap();

    response.headers
            .get::<SetCookie>()
            .expect("no login cookies")
            .apply_to_cookie_jar(&mut cookie_jar);

    let decoded_body = {
        let mut buf = Vec::new();
        response.read_to_end(&mut buf).unwrap();
        WINDOWS_1251.decode(&*buf, DecoderTrap::Replace).unwrap()
    };

    let action = parser.parse(action_re.captures(&*decoded_body)
                                       .expect("no action URL found in login form")
                                       .at(1)
                                       .unwrap())
                       .unwrap();
    let form = form_urlencoded::serialize(input_re.captures_iter(&*decoded_body)
                                                  .map(|c| (c.at(1).unwrap(), c.at(2).unwrap())));
    debug!("got second stage form: {} {:?}", action, form);

    debug!("running second stage...");
    client.set_redirect_policy(RedirectPolicy::FollowNone);
    let response = client.post(action)
                         .body(&*form)
                         .header(Cookie::from_cookie_jar(&cookie_jar))
                         .header(ContentType("application/x-www-form-urlencoded".parse().unwrap()))
                         .header(UserAgent(USER_AGENT.to_string()))
                         .header(Referer(LOGIN_URL.to_string()))
                         .send()
                         .unwrap();

    response.headers
            .get::<SetCookie>()
            .expect("not session cookies")
            .apply_to_cookie_jar(&mut cookie_jar);
    debug!("logged in as {}", login);

    cookie_jar
}

enum RssState {
    Init,
    InChannel,
    InItem,
    InTitle,
    InLink,
}

fn get_torrent_urls(cookie_jar: &CookieJar,
                    include: &[String],
                    exclude: &[String])
                    -> Vec<(String, String)> {
    debug!("trying to get RSS feed...");
    let url = format!("{}{}", BASE_URL, "rssdd.xml");

    debug!("sending request...");
    let mut body = Vec::new();
    let client = Client::new();
    client.get(&*url)
          .header(UserAgent(USER_AGENT.to_string()))
          .send()
          .unwrap()
          .read_to_end(&mut body)
          .unwrap();

    debug!("parsing response...");
    let decoded_body = WINDOWS_1251.decode(&*body, DecoderTrap::Replace).unwrap();
    let reader = EventReader::new(decoded_body.as_bytes());

    let mut state = RssState::Init;
    let mut result = Vec::new();
    let mut needed = false;
    let mut title = "".to_string();

    for ev in reader {
        match ev.unwrap() {
            XmlEvent::StartElement { name: OwnedName { ref local_name, .. }, .. } => {
                match (&state, &**local_name) {
                    (&RssState::Init, "channel") => state = RssState::InChannel,
                    (&RssState::InChannel, "item") => state = RssState::InItem,
                    (&RssState::InItem, "title") => state = RssState::InTitle,
                    (&RssState::InItem, "link") => state = RssState::InLink,
                    _ => (),
                }
            }
            XmlEvent::EndElement { name: OwnedName { ref local_name, .. } } => {
                match (&state, &**local_name) {
                    (&RssState::InChannel, "channel") => state = RssState::Init,
                    (&RssState::InItem, "item") => state = RssState::InChannel,
                    (&RssState::InTitle, "title") => state = RssState::InItem,
                    (&RssState::InLink, "link") => state = RssState::InItem,
                    _ => (),
                }
            }
            XmlEvent::Characters(ref value) => {
                match state {
                    RssState::InTitle => {
                        needed = include.iter().find(|v| value.contains(&***v)).is_some() &&
                                 !exclude.iter().find(|v| value.contains(&***v)).is_some();

                        if needed {
                            title = value.clone();
                        }
                    }
                    RssState::InLink if needed => {
                        result.push((title.clone(), extract_torrent_link(cookie_jar, value.replace("/download.php?", "/details.php?").rsplitn(1, '&').last().expect("torrent URL parse failed"))));
                    }
                    _ => (),
                }
            }
            _ => (),
        }
    }

    debug!("feed parsed for {} links: {:?}", result.len(), result);
    result
}

fn extract_torrent_link(cookie_jar: &CookieJar, details_url: &str) -> String {
    debug!("extracting torrent link for {}", details_url);
    let a_download_tag_re = Regex::new(r#"<a href="javascript:\{\};" onMouseOver="setCookie\('(\w+)','([a-f0-9]+)'\)" class="a_download" onClick="ShowAllReleases\('([0-9]+)','([0-9.]+)','([0-9]+)'\)"></a>"#).unwrap();
    let torrent_link_re = Regex::new(r#"href="(http://tracktor\.in/td\.php\?s=[^"]+)""#).unwrap();

    let mut client = Client::new();
    client.set_redirect_policy(RedirectPolicy::FollowAll);

    debug!("fetching details page...");
    let mut body = Vec::new();
    client.get(details_url)
          .header(Cookie::from_cookie_jar(cookie_jar))
          .header(UserAgent(USER_AGENT.to_string()))
          .header(Referer(BASE_URL.to_string()))
          .send()
          .unwrap()
          .read_to_end(&mut body)
          .unwrap();

    debug!("parsing details page...");
    let decoded_body = WINDOWS_1251.decode(&*body, DecoderTrap::Replace).unwrap();

    let a_download_tag = a_download_tag_re.captures(&*decoded_body).unwrap();
    let (href, cookie_name, cookie_value) = (format!("{}nrdr.php?c={}&s={}&e={}",
                                                     BASE_URL,
                                                     a_download_tag.at(3).unwrap(),
                                                     a_download_tag.at(4).unwrap(),
                                                     a_download_tag.at(5).unwrap()),
                                             format!("{}_2", a_download_tag.at(1).unwrap()),
                                             a_download_tag.at(2).unwrap().to_string());

    cookie_jar.add(CookiePair::new(cookie_name, cookie_value));

    debug!("fetching tracker page...");
    let mut body = Vec::new();
    client.get(&*href)
          .header(Cookie::from_cookie_jar(cookie_jar))
          .header(UserAgent(USER_AGENT.to_string()))
          .header(Referer(details_url.to_string()))
          .send()
          .unwrap()
          .read_to_end(&mut body)
          .unwrap();

    debug!("parsing tracker page...");
    let decoded_body = WINDOWS_1251.decode(&*body, DecoderTrap::Replace).unwrap();
    torrent_link_re.captures(&*decoded_body).unwrap().at(1).unwrap().to_string()
}

struct TransmissionAPI {
    token: TransmissionSessionId,
    tag: u32,
}

#[derive(Debug, Clone)]
struct TransmissionSessionId(pub String);

impl Header for TransmissionSessionId {
    #[allow(unused_variables)]
    fn header_name() -> &'static str {
        "X-Transmission-Session-Id"
    }

    fn parse_header(raw: &[Vec<u8>]) -> Result<TransmissionSessionId, HttpError> {
        Ok(TransmissionSessionId(String::from_utf8_lossy(&*raw[0]).into_owned()))
    }
}

impl HeaderFormat for TransmissionSessionId {
    fn fmt_header(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let TransmissionSessionId(ref value) = *self;
        fmt.write_str(&**value)
    }
}

impl TransmissionAPI {
    pub fn new() -> TransmissionAPI {
        TransmissionAPI {
            token: TransmissionSessionId(String::new()),
            tag: 0,
        }
    }

    pub fn add_torrent(&mut self, url: &str, download_dir: Option<&str>) -> bool {
        debug!("adding {} ({:?}) to torrents queue", url, download_dir);
        let client = Client::new();

        loop {
            self.tag = self.tag + 1;
            debug!("sending request with tag {}...", self.tag);

            let resp = format!(r#"{{"tag":"{}","method":"torrent-add","arguments":{{"filename":"{}","download-dir":"{}"}}}}"#, self.tag, url, download_dir.unwrap_or(""));
            let mut resp = client.post(TRANSMISSION_URL)
                                 .body(&*resp)
                                 .header(self.token.clone())
                                 .header(ContentType("application/json".parse().unwrap()))
                                 .send()
                                 .unwrap();

            match resp.status {
                StatusCode::Ok => {
                    debug!("torrent added");
                    let mut buf = String::new();
                    resp.read_to_string(&mut buf).unwrap();
                    return buf.contains("torrent-added");
                }
                StatusCode::Conflict => {
                    debug!("session id update");
                    self.token = resp.headers.get::<TransmissionSessionId>().unwrap().clone();
                }
                code @ _ => {
                    panic!("unexpected error code {} for torrent {}", code, url);
                }
            }
        }
    }
}

fn main() {
    debug!("1. loading configs...");
    let config: Config = utils::load_config("lostfilm/config.toml").expect("config file missing");
    let download_dir = config.download_dir.as_ref().map(|v| &**v);

    debug!("2. initializing api objects...");
    let pbcfg = utils::load_config::<PbConfig>("pushbullet/config.toml")
                    .expect("pushbullet config missing");
    let mut pbapi = PbAPI::new(&*pbcfg.access_token);
    let mut trans = TransmissionAPI::new();

    debug!("3. logging in to lostfilm.tv...");
    let cookie_jar = login(&*config.username, &*config.password);

    debug!("4. checking lostfilm.tv for new shows...");
    let urls = get_torrent_urls(&cookie_jar, &*config.include, &*config.exclude);

    debug!("5. adding found torrents to transmission...");
    for (title, url) in urls.into_iter() {
        if trans.add_torrent(&*url, download_dir) {
            notify(&mut pbapi, pbcfg.device_iden.clone(), &*title, &*url);
        }
    }

    debug!("all done!");
}
