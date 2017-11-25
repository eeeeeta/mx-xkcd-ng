#![feature(proc_macro, conservative_impl_trait, generators, try_trait, slice_patterns, advanced_slice_patterns)]
extern crate glitch_in_the_matrix as gm;
extern crate futures_await as futures;
extern crate tokio_core;
extern crate dotenv;
extern crate serde_json;
extern crate serde;
extern crate hyper;
extern crate gm_boilerplate;
extern crate image;
#[macro_use] extern crate serde_derive;
extern crate tokio_timer;
extern crate chrono;

use futures::prelude::*;
use dotenv::dotenv;
use tokio_core::reactor::{Handle, Core};
use gm::{MatrixClient, MatrixFuture};
use gm::types::messages::Message;
use gm::types::sync::SyncReply;
use gm::types::content::{Content};
use gm::room::{Room, RoomExt};
use gm::types::messages::ImageInfo;
use gm::errors::*;
use gm::types::events::Event;
use std::env;
use std::rc::Rc;
use std::cell::RefCell;
use gm_boilerplate::{MatrixBot, BoilerplateConfig, make_bot_future, sync_boilerplate};

pub type Hyper = Rc<RefCell<gm::http::MatrixHyper>>;
pub type Mx = Rc<RefCell<MatrixClient>>;

pub mod xkcd;
pub mod lolcount;
pub mod inspirobot;
pub mod wfrog;

pub fn validate_response(resp: &hyper::Response) -> MatrixResult<()> {
    if !resp.status().is_success() {
        Err(resp.status().canonical_reason().unwrap_or("Unknown response failure").to_string().into())
    }
    else {
        Ok(())
    }
}
pub struct XkcdBot {
    mx: Option<Mx>,
    hyper: Option<Hyper>,
    hdl: Handle,
    wfrog_rooms: Rc<RefCell<Vec<Room<'static>>>>
}
impl XkcdBot {
    fn text_to_cmd(body: &str) -> Option<Command> {
        let args = body.split(" ").map(|x| x.to_lowercase()).collect::<Vec<String>>();
        let args: Vec<&str> = args.iter().map(|s| &**s).collect();
        if args.contains(&"lol") {
            return Some(Command::IncrementLolcount)
        }
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
            &["mittwoch"] => Some(Command::WfrogState),
            &["mittwoch", "on"] | &["mittwoch", "enable"] => Some(Command::WfrogDisableEnable(true)),
            &["mittwoch", "off"] | &["mittwoch", "disable"] => Some(Command::WfrogDisableEnable(false)),
            &["mittwoch", "text", ref x..] => Some(Command::WfrogSetText(x.join(" ").into())),
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
    fn message_to_cmds(mx: Mx, room: Room<'static>, sender: String, body: &str) -> MatrixFuture<Vec<Command>> {
        if let Some(cmd) = Self::text_to_cmd(body) {
            Box::new(async_block! {
                let pl = await!(room.cli(&mut mx.borrow_mut())
                                .get_user_power_level(sender))?;
                Ok(match cmd {
                    cmd @ Command::WfrogDisableEnable(..) |
                    cmd @ Command::WfrogSetText(..) => {
                        let cmd = if pl >= 20 { cmd } else { Command::PowerLevelFail };
                        vec![cmd]
                    },
                    cmd @ Command::SetLolcount(..) => {
                        let cmd = if pl >= 50 { cmd } else { Command::PowerLevelFail };
                        vec![cmd]
                    },
                    cmd => vec![cmd]
                })
            })
        }
        else {
            Box::new(async_block! {
                Ok(vec![])
            })
        }
    }
    #[async]
    fn on_mittwoch(mx: Mx, room: Room<'static>, url: String, info: ImageInfo, sender: String) -> MatrixResult<Vec<Command>> {
        let mut ret = vec![];
        let pl = await!(room.cli(&mut mx.borrow_mut())
                        .get_user_power_level(sender))?;
        if pl > 20 {
            ret.push(Command::WfrogEnable(url, info));
        }
        else {
            ret.push(Command::PowerLevelFail)
        }
        Ok(ret)
    }
}
impl MatrixBot for XkcdBot {
    type Command = Command;
    type SyncFuture = MatrixFuture<Vec<(Room<'static>, Self::Command)>>;
    type CmdFuture = MatrixFuture<()>;
    type ErrorFuture = MatrixFuture<()>;
    fn on_login(&mut self, mx: Mx) {
        println!("[+] Bot connected.");
        wfrog::wfrog_init(mx.clone(), &self.hdl, self.wfrog_rooms.clone());
        self.hyper = Some(Rc::new(RefCell::new(mx.borrow_mut().get_hyper().clone())));
        self.mx = Some(mx);
    }
    fn on_sync(&mut self, reply: SyncReply) -> Self::SyncFuture {
        for (room, _) in reply.rooms.join.iter() {
            {
                let mut wfr = self.wfrog_rooms.borrow_mut();
                if !wfr.contains(room) {
                    wfr.push(room.clone());
                }
            }
        }
        fn hdl(mx: Mx, r: &Room<'static>, e: &Event) -> MatrixFuture<Vec<Command>> {
            if let Event::Full(ref meta, ref content) = *e {
                if let Content::RoomMessage(ref m) = *content {
                    if let Message::Text { ref body, .. } = *m {
                        return XkcdBot::message_to_cmds(mx, r.to_owned(), meta.sender.to_owned(), body);
                    }
                    else if let Message::Image { ref body, ref info, ref url, .. } = *m {
                        if body == "mittwoch.png" && info.is_some() {
                            return Box::new(
                                XkcdBot::on_mittwoch(mx, r.clone(), url.clone(), info.clone().unwrap(), meta.sender.clone())
                            );
                        }
                    }
                }
            }
            Box::new(async_block! {
                Ok(vec![])
            })
        };
        sync_boilerplate(self.mx.clone().unwrap(), reply, hdl)
    }
    fn on_command(&mut self, room: Room<'static>, cmd: Self::Command) -> MatrixFuture<()> {
        let mx = self.mx.clone().unwrap();
        let hyper = self.hyper.clone().unwrap();
        Box::new(async_block! {
            match cmd {
                Command::Ping => {
                    await!(room.cli(&mut mx.borrow_mut()).send_simple("ohai"))?;
                },
                Command::Comic(c) => {
                    let c = await!(xkcd::fetch_comic(hyper, c))?;
                    await!(xkcd::send_comic(mx, room, c))?;
                },
                Command::Inspirobot => {
                    await!(inspirobot::inspirobot(hyper, mx, room))?;
                },
                Command::GetLolcount => {
                    let lols = await!(lolcount::get_lols(mx.clone(), room.clone()))?;
                    await!(room.cli(&mut mx.borrow_mut())
                           .send_simple(format!("lolcount: {}", lols)))?;
                },
                Command::IncrementLolcount => {
                    let lols = await!(lolcount::get_lols(mx.clone(), room.clone()))?;
                    await!(lolcount::set_lols(mx.clone(), room.clone(), lols + 1))?;
                    await!(room.cli(&mut mx.borrow_mut())
                           .send_simple(format!("lolcount: {}", lols + 1)))?;
                },
                Command::WfrogState => {
                    await!(wfrog::wfrog_explain_state(mx, room))?;
                },
                Command::SetLolcount(n) => {
                    await!(lolcount::set_lols(mx.clone(), room.clone(), n))?;
                    await!(room.cli(&mut mx.borrow_mut())
                           .send_simple(format!("lolcount updated - new value: {}", n)))?;
                },
                Command::WfrogDisableEnable(n) => {
                    await!(wfrog::wfrog_disable_enable(mx, room, n))?;
                },
                Command::WfrogSetText(n) => {
                    await!(wfrog::wfrog_set_text(mx, room, n))?;
                },
                Command::WfrogEnable(body, info) => {
                    await!(wfrog::wfrog_set_image(mx, room, body, info))?;
                },
                Command::PowerLevelFail => {
                    await!(room.cli(&mut mx.borrow_mut())
                           .send_simple("(You can't tell me what to do!)"))?;
                },
                Command::ParseFail => {
                    await!(room.cli(&mut mx.borrow_mut())
                           .send_simple("(Parsing failed.)"))?;
                }
            }
            Ok(())
        })
    }
    fn on_error(&mut self, room: Option<Room<'static>>, error: MatrixError) -> Self::ErrorFuture {
        let mx = self.mx.clone().unwrap();
        Box::new(async_block! {
            if let Some(rm) = room {
                await!(rm.cli(&mut mx.borrow_mut())
                       .send_simple(format!("[error] {}", error)))?;
            }
            Ok(())
        })
    }
}
#[derive(Clone)]
pub enum Command {
    Ping,
    Comic(Option<u32>),
    Inspirobot,
    SetLolcount(u32),
    GetLolcount,
    IncrementLolcount,
    WfrogState,
    WfrogDisableEnable(bool),
    WfrogSetText(String),
    WfrogEnable(String, ImageInfo),
    ParseFail,
    PowerLevelFail
}
fn main() {
    dotenv().unwrap();
    let server = env::var("SERVER").expect("set the server variable");
    let username = env::var("USERNAME").expect("set the username variable");
    let password = env::var("PASSWORD").expect("set the password variable");
    let cfg = BoilerplateConfig { server, username, password };
    let mut core = Core::new().unwrap();
    let hdl = core.handle();
    let wfrog_rooms = Rc::new(RefCell::new(vec![]));
    let bot = XkcdBot { mx: None, hyper: None, hdl: hdl.clone(), wfrog_rooms };
    let fut = make_bot_future(hdl, cfg, bot);
    println!("[+] Starting bot!");
    core.run(fut).unwrap();
}
