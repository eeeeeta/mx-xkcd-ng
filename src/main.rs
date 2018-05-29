#![feature(proc_macro, proc_macro_non_items, generators, try_trait, slice_patterns)]
extern crate glitch_in_the_matrix as gm;
extern crate futures_await as futures;
extern crate tokio_core;
extern crate dotenv;
extern crate serde_json;
extern crate serde;
extern crate hyper;
extern crate image;
#[macro_use] extern crate serde_derive;
extern crate tokio_timer;
extern crate chrono;
#[macro_use] extern crate failure;
extern crate regex;
#[macro_use] extern crate lazy_static;

use futures::prelude::*;
use dotenv::dotenv;
use tokio_core::reactor::{Handle, Core};
use gm::{MatrixClient};
use gm::types::messages::Message;
use gm::types::content::{Content};
use gm::room::{Room, RoomExt};
use gm::sync::SyncStream;
use gm::request::MatrixRequestable;
use std::env;
use std::rc::Rc;
use std::cell::RefCell;
use regex::Regex;

pub type Hyper = Rc<RefCell<gm::MatrixHyper>>;
pub type Mx = Rc<RefCell<MatrixClient>>;
pub type Result<T> = ::std::result::Result<T, failure::Error>;

pub mod xkcd;
pub mod lolcount;
pub mod inspirobot;

pub fn validate_response(resp: &hyper::Response) -> Result<()> {
    if !resp.status().is_success() {
        Err(format_err!("{}", resp.status().canonical_reason().unwrap_or("Unknown response failure")))
    }
    else {
        Ok(())
    }
}
pub struct XkcdBot {
    mx: Mx,
    hyper: Hyper,
    hdl: Handle
}
#[derive(Clone, Debug)]
pub enum Command {
    Ping,
    Comic(Option<u32>),
    Inspirobot,
    SetLolcount(u32),
    GetLolcount,
    IncrementLolcount,
    ParseFail
}
impl XkcdBot {
    fn process_command(&self, sender: String, room: Room<'static>, cmd: Command) -> impl Future<Item = (), Error = ::failure::Error> {
        let mut mx = self.mx.clone();
        let h = self.hyper.clone();
        async_block! {
            use self::Command::*;

            match cmd {
                Ping => {
                    await!(room.cli(&mut mx).send_simple("Pong!"))?;
                },
                Comic(n) => {
                    let c = await!(xkcd::fetch_comic(h, n))?;
                    await!(xkcd::send_comic(mx, room, c))?;
                },
                Inspirobot => {
                    await!(inspirobot::inspirobot(h, mx, room))?;
                },
                GetLolcount => {
                    let lols = await!(lolcount::get_lols(mx.clone(), room.clone()))?;
                    await!(room.cli(&mut mx).send_simple(format!("lolcount: {}", lols)))?;
                },
                IncrementLolcount => {
                    let lols = await!(lolcount::get_lols(mx.clone(), room.clone()))?;
                    await!(lolcount::set_lols(mx.clone(), room.clone(), lols + 1))?;
                    await!(room.cli(&mut mx).send_simple(format!("lolcount: {}", lols + 1)))?;
                },
                SetLolcount(n) => {
                    let pl = await!(room.cli(&mut mx).get_user_power_level(sender))?;
                    if pl < 50 {
                        Err(format_err!("You require a power level above 50 to do that."))?;
                    }
                    await!(lolcount::set_lols(mx.clone(), room.clone(), n))?;
                    await!(room.cli(&mut mx).send_simple(format!("lolcount updated - new value: {}", n)))?;

                },
                ParseFail => Err(format_err!("Failed to parse command."))?
            }
            Ok(())
        }
    }
    fn message_to_command(m: &str) -> Option<Command> {
        lazy_static! {
            static ref LOL_REGEX: Regex = Regex::new(r"(?i)\bl([oeu]l)+\b").unwrap();
        }
        if LOL_REGEX.is_match(m) {
            return Some(Command::IncrementLolcount);
        }
        let args = m.split(" ").collect::<Vec<_>>();
        match &args as &[&str] {
            &["xkcd", "ping"] => Some(Command::Ping),
            &["xkcd", "latest"] => Some(Command::Comic(None)),
            &["xkcd", n] => {
                if let Ok(n) = n.parse::<u32>() {
                    Some(Command::Comic(Some(n)))
                }
                else {
                    Some(Command::ParseFail)
                }
            },
            &["inspirobot"] => Some(Command::Inspirobot),
            &["lolcount"] => Some(Command::GetLolcount),
            &["lolcount", "set", n] => {
                if let Ok(n) = n.parse::<u32>() {
                    Some(Command::SetLolcount(n))
                }
                else {
                    Some(Command::ParseFail)
                }
            },
            _ => None
        }
    }
}
fn main() -> Result<()> {
    println!("[+] mx-xkcd-ng, an eta project");
    println!("[+] Reading environment variables");
    dotenv().unwrap();
    let server = env::var("SERVER").expect("set the SERVER variable");
    let token = env::var("ACCESS_TOKEN").expect("set the ACCESS_TOKEN variable");
    println!("[+] Initialising tokio");
    let mut core = Core::new()?;
    let hdl = core.handle();
    println!("[+] Logging in with token");
    let mx = core.run(MatrixClient::new_from_access_token(&token, &server, &hdl))?;
    let hyper = mx.get_hyper();
    let mx = Rc::new(RefCell::new(mx));
    let ss = SyncStream::new(mx.clone());
    let mut bot = XkcdBot {
        mx,
        hyper: Rc::new(RefCell::new(hyper)),
        hdl
    };
    let fut = ss.skip(1).for_each(|sync| {
        for (room, evt) in sync.iter_events() {
            if let Some(ref rd) = evt.room_data {
                if rd.sender == bot.mx.borrow().get_user_id() {
                    continue;
                }
                {
                    let mut rc = room.cli(&mut bot.mx);
                    let fut = rc.read_receipt(&rd.event_id)
                        .map(|_| ()).map_err(|e| {
                            eprintln!("[!] Error sending read receipt: {}", e);
                        });
                    bot.hdl.spawn(fut);
                }
                if let Content::RoomMessage(ref m) = evt.content {
                    if let Message::Text { ref body, .. } = *m {
                        if let Some(cmd) = XkcdBot::message_to_command(body) {
                            println!("[*] Processing command: {} -> {:?}", body, cmd);
                            let room = room.clone();
                            let mut mx = bot.mx.clone();
                            let fut = bot.process_command(rd.sender.clone(), room.clone(), cmd)
                                .map_err(move |e| {
                                    eprintln!("[!] Error processing command: {}", e);
                                    let _ = room.cli(&mut mx).send_simple(format!("[!] error: {}", e));
                                });
                            bot.hdl.spawn(fut);
                        }
                    }
                }
            }
        }
        Ok(())
    });
    println!("[+] Starting bot!");
    core.run(fut)?;
    Ok(())
}
