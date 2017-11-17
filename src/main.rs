#![feature(proc_macro, conservative_impl_trait, generators, try_trait, slice_patterns, advanced_slice_patterns)]
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

use futures::{Future, Stream};
use futures::prelude::*;
use dotenv::dotenv;
use tokio_core::reactor::Core;
use gm::{MatrixClient, MatrixFuture};
use gm::types::messages::Message;
use gm::types::content::{Content};
use gm::room::{Room, RoomExt};
use gm::types::messages::ImageInfo;
use gm::errors::*;
use gm::types::events::Event;
use std::env;
use std::rc::Rc;
use std::cell::RefCell;

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
#[derive(Clone)]
enum Command {
    Ping,
    Comic(Option<u32>),
    Inspirobot,
    SetLolcount(u32),
    GetLolcount,
    IncrementLolcount,
    WfrogState,
    WfrogDisableEnable(bool),
    WfrogSetText(String)
}
#[async]
fn on_mittwoch(mx: Mx, room: Room<'static>, url: String, info: ImageInfo, sender: String) -> MatrixResult<()> {
    let pl = await!(room.cli(&mut mx.borrow_mut())
                    .get_user_power_level(sender))?;
    if pl > 20 {
        let _ = await!(wfrog::wfrog_set_image(mx.clone(), room.clone(), url, info));
        let _ = await!(room.cli(&mut mx.borrow_mut())
                       .send_simple("Wednesday frog activated!"))?;
    }
    else {
        let _ = await!(room.cli(&mut mx.borrow_mut())
                       .send_simple("(Wednesday frog didn't activate, because you don't have the required permissions.)"))?;
    }
    Ok(())
}
#[async]
fn on_msg(mx: Mx, hyper: Hyper, room: Room<'static>, body: String, sender: String) -> MatrixResult<()> {
    if body.len() < 1 { return Ok(()); }
    let cmd = {
        let args = body.split(" ").map(|x| x.to_lowercase()).collect::<Vec<String>>();
        let args: Vec<&str> = args.iter().map(|s| &**s).collect();
        if args.contains(&"lol") {
            Some(Command::IncrementLolcount)
        }
        else {
            match &args as &[&str] {
                &["xkcd", "ping"] => Some(Command::Ping),
                &["xkcd", "latest"] => Some(Command::Comic(None)),
                &["xkcd", n] => {
                    let n = match n.parse::<u32>() {
                        Ok(n) => n,
                        Err(e) => return Err(e.to_string().into())
                    };
                    Some(Command::Comic(Some(n)))
                },
                &["inspirobot"] => Some(Command::Inspirobot),
                &["lolcount"] => Some(Command::GetLolcount),
                &["mittwoch"] => Some(Command::WfrogState),
                &["mittwoch", "on"] | &["mittwoch", "enable"] => Some(Command::WfrogDisableEnable(true)),
                &["mittwoch", "off"] | &["mittwoch", "disable"] => Some(Command::WfrogDisableEnable(false)),
                &["mittwoch", "text", ref x..] => Some(Command::WfrogSetText(x.join(" ").into())),
                &["lolcount", "set", n] => {
                    let n = match n.parse::<u32>() {
                        Ok(n) => n,
                        Err(e) => return Err(e.to_string().into())
                    };
                    Some(Command::SetLolcount(n))
                },
                _ => None
            }
        }
    };
    if let Some(cmd) = cmd {
        match cmd {
            Command::Ping => {
                await!(room.cli(&mut mx.borrow_mut()).send_simple("ohai"))?;
            },
            Command::Comic(c) => {
                let c = await!(xkcd::fetch_comic(hyper.clone(), c))?;
                await!(xkcd::send_comic(mx.clone(), room.clone(), c))?;
            },
            Command::Inspirobot => {
                await!(inspirobot::inspirobot(hyper.clone(), mx.clone(), room.clone()))?;
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
                await!(wfrog::wfrog_explain_state(mx.clone(), room.clone()))?;
            },
            cmd => {
                let pl = await!(room.cli(&mut mx.borrow_mut())
                                .get_user_power_level(sender))?;
                match cmd {
                    Command::SetLolcount(n) => {
                        if pl > 50 {
                            await!(lolcount::set_lols(mx.clone(), room.clone(), n))?;
                            await!(room.cli(&mut mx.borrow_mut())
                                   .send_simple(format!("lolcount updated - new value: {}", n)))?;
                            return Ok(());
                        }
                    },
                    Command::WfrogDisableEnable(n) => {
                        if pl > 50 {
                            await!(wfrog::wfrog_disable_enable(mx.clone(), room.clone(), n))?;
                            return Ok(());
                        }
                    },
                    Command::WfrogSetText(n) => {
                        if pl > 50 {
                            await!(wfrog::wfrog_set_text(mx.clone(), room.clone(), n))?;
                            return Ok(());
                        }
                    },
                    _ => {}
                }
                await!(room.cli(&mut mx.borrow_mut())
                       .send_simple("You can't tell me what to do!"))?;
            }
        }
    }
    Ok(())
}
fn main() {
    dotenv().unwrap();
    let server = env::var("SERVER").expect("set the server variable");
    let username = env::var("USERNAME").expect("set the username variable");
    let password = env::var("PASSWORD").expect("set the password variable");
    let mut core = Core::new().unwrap();
    let hdl = core.handle();
    let mut mx = core.run(MatrixClient::login(&username, &password, &server, &hdl)).unwrap();
    println!("[+] Connected to {} as {}", server, username);
    let ss = mx.get_sync_stream();
    let hyper = Rc::new(RefCell::new(mx.get_hyper().clone()));
    let mx = Rc::new(RefCell::new(mx));
    let wfrog_rooms = Rc::new(RefCell::new(vec![]));
    wfrog::wfrog_init(mx.clone(), &hdl, wfrog_rooms.clone());
    let fut = ss.skip(1).for_each(|sync| {
        let mut futs: Vec<MatrixFuture<()>> = vec![];
        for (room, events) in sync.rooms.join {
            {
                let mut wfr = wfrog_rooms.borrow_mut();
                if !wfr.contains(&room) {
                    wfr.push(room.clone());
                }
            }
            for ev in events.timeline.events {
                if let Event::Full(meta, content) = ev {
                    {
                        let mut mx = mx.borrow_mut();
                        // only echo messages from other users
                        if meta.sender == mx.user_id() {
                            continue;
                        }
                        // tell the server we have read the event
                        let mut rc = room.cli(&mut mx);
                        futs.push(Box::new(rc.read_receipt(&meta.event_id).map(|_| ())));
                    }
                    if let Content::RoomMessage(m) = content {
                        if let Message::Text { body, .. } = m {
                            let mx2 = mx.clone();
                            let rc = room.clone();
                            let fut = on_msg(mx.clone(), hyper.clone(), room.clone(), body, meta.sender.clone())
                                .or_else(move |err| {
                                    let mut mx = mx2.borrow_mut();
                                    let mut rc = rc.cli(&mut mx);
                                    rc.send_simple(format!("[error] {}", err))
                                        .map(|_| ())
                                });
                            futs.push(Box::new(fut));
                        }
                        else if let Message::Image { body, info, url, .. } = m {
                            if body == "mittwoch.png" && info.is_some() {
                                let fut = on_mittwoch(mx.clone(), room.clone(), url, info.unwrap(), meta.sender.clone());
                                futs.push(Box::new(fut));
                            }
                        }
                    }
                }
            }
        }
        futures::future::join_all(futs.into_iter()).map(|_| ())
    });
    println!("[+] Starting bot!");
    core.run(fut).unwrap();
}
