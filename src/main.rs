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

use futures::{Future, Stream};
use futures::prelude::*;
use dotenv::dotenv;
use tokio_core::reactor::Core;
use gm::{MatrixClient, MatrixFuture};
use gm::http::ContentType;
use gm::types::messages::{Message, ImageInfo};
use gm::types::content::{Content};
use hyper::{Chunk, Uri};
use gm::room::Room;
use gm::errors::*;
use gm::types::events::Event;
use std::env;
use std::rc::Rc;
use std::cell::RefCell;
use image::GenericImage;

#[derive(Deserialize)]
struct Comic {
    num: u32,
    img: String,
    #[serde(skip_deserializing)]
    data: Option<Chunk>,
    alt: String,
    title: String
}
#[derive(Serialize, Deserialize, Default)]
struct Lolcount {
    lol_count: u32
}
type Hyper = Rc<RefCell<gm::http::MatrixHyper>>;
type Mx = Rc<RefCell<MatrixClient>>;

fn validate_response(resp: &hyper::Response) -> MatrixResult<()> {
    if !resp.status().is_success() {
        Err(resp.status().canonical_reason().unwrap_or("Unknown response failure").to_string().into())
    }
    else {
        Ok(())
    }
}
#[async]
fn fetch_comic(h: Hyper, n: Option<u32>) -> MatrixResult<Comic> {
    let n = match n {
        Some(n) => format!("{}/", n),
        None => "".into()
    };
    let uri: Uri = format!("https://xkcd.com/{}info.0.json", n).parse()?;
    let resp = await!(h.borrow_mut().get(uri))?;
    validate_response(&resp)?;
    let body = await!(resp.body().concat2())?;
    let mut c: Comic = serde_json::from_slice(&body)?;
    let img: Uri = c.img.parse()?;
    let resp = await!(h.borrow_mut().get(img))?;
    validate_response(&resp)?;
    let body = await!(resp.body().concat2())?;
    c.data = Some(body);
    Ok(c)
}
#[async]
fn send_comic(cli: Mx, r: Room<'static>, mut c: Comic) -> MatrixResult<()> {
    let data = c.data.take().unwrap();
    let img = image::load_from_memory(&data as &[u8])
        .map_err(|e| e.to_string())?;
    let info = ImageInfo {
        mimetype: "image/png".into(),
        h: img.height(),
        w: img.width(),
        size: data.len() as _
    };
    let rpl = await!(cli.borrow_mut().upload(data, ContentType::png()))?;
    await!(r.cli(&mut cli.borrow_mut())
           .send_simple(format!("{}: {}", c.num, c.title)))?;
    let img = Message::Image {
        body: c.title.clone(),
        info: Some(info),
        thumbnail_info: None,
        thumbnail_url: None,
        url: rpl.content_uri
    };
    await!(r.cli(&mut cli.borrow_mut())
           .send(img))?;
    await!(r.cli(&mut cli.borrow_mut())
           .send_simple(format!("Alt text: {}", c.alt)))?;
    Ok(())
}
#[async]
fn inspirobot(h: Hyper, cli: Mx, r: Room<'static>) -> MatrixResult<()> {
    let uri: Uri = "http://inspirobot.me/api?generate=true".parse()?;
    let resp = await!(h.borrow_mut().get(uri))?;
    validate_response(&resp)?;
    let body = await!(resp.body().concat2())?;
    let uri: Uri = String::from_utf8(body.to_vec())
        .map_err(|e| e.to_string())?.parse()?;
    let resp = await!(h.borrow_mut().get(uri))?;
    validate_response(&resp)?;
    let body = await!(resp.body().concat2())?;
    let img = image::load_from_memory_with_format(
        &body as &[u8],
        image::ImageFormat::JPEG
    ).map_err(|e| e.to_string())?;
    let info = ImageInfo {
        mimetype: "image/jpeg".into(),
        h: img.height(),
        w: img.width(),
        size: body.len() as _
    };
    let rpl = await!(cli.borrow_mut().upload(body, ContentType::jpeg()))?;
    let img = Message::Image {
        body: "inspirobot".into(),
        info: Some(info),
        thumbnail_info: None,
        thumbnail_url: None,
        url: rpl.content_uri
    };
    await!(r.cli(&mut cli.borrow_mut())
           .send(img))?;
    Ok(())
}
#[async]
fn get_lols(cli: Mx, r: Room<'static>) -> MatrixResult<u32> {
    let res = await!(r.cli(&mut cli.borrow_mut())
                     .get_state::<Lolcount>("org.eu.theta.lolcount", None));
    match res {
        Ok(l) => Ok(l.lol_count),
        Err(e) => {
            if let &MatrixErrorKind::BadRequest(ref brk) = e.kind() {
                if brk.errcode == "M_NOT_FOUND" {
                    return Ok(0)
                }
            }
            Err(e)
        }
    }
}
#[async]
fn set_lols(cli: Mx, r: Room<'static>, lc: u32) -> MatrixResult<()> {
    let lolcount = Lolcount { lol_count: lc };
    await!(r.cli(&mut cli.borrow_mut())
           .set_state("org.eu.theta.lolcount", None, lolcount))?;
    Ok(())
}
#[derive(Copy, Clone)]
enum Command {
    Ping,
    Comic(Option<u32>),
    Inspirobot,
    SetLolcount(u32),
    GetLolcount,
    IncrementLolcount
}
#[async]
fn on_msg(mx: Mx, hyper: Hyper, room: Room<'static>, body: String, admin: bool) -> MatrixResult<()> {
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
    match cmd {
        Some(Command::Ping) => {
            await!(room.cli(&mut mx.borrow_mut()).send_simple("ohai"))?;
        },
        Some(Command::Comic(c)) => {
            let c = await!(fetch_comic(hyper.clone(), c))?;
            println!("got comic #{}", c.num);
            await!(send_comic(mx.clone(), room.clone(), c))?;
        },
        Some(Command::Inspirobot) => {
            await!(inspirobot(hyper.clone(), mx.clone(), room.clone()))?;
        },
        Some(Command::GetLolcount) => {
            let lols = await!(get_lols(mx.clone(), room.clone()))?;
            await!(room.cli(&mut mx.borrow_mut())
                  .send_simple(format!("lolcount: {}", lols)))?;
        },
        Some(Command::IncrementLolcount) => {
            let lols = await!(get_lols(mx.clone(), room.clone()))?;
            await!(set_lols(mx.clone(), room.clone(), lols + 1))?;
            await!(room.cli(&mut mx.borrow_mut())
                   .send_simple(format!("lolcount: {}", lols + 1)))?;
        },
        Some(Command::SetLolcount(n)) => {
            if !admin {
                await!(room.cli(&mut mx.borrow_mut())
                       .send_simple("You can't tell me what to do!"))?;
            }
            else {
                await!(set_lols(mx.clone(), room.clone(), n))?;
                await!(room.cli(&mut mx.borrow_mut())
                       .send_simple(format!("lolcount updated - new value: {}", n)))?;
            }
        },
        _ => {}
    }
    Ok(())
}
fn main() {
    dotenv().unwrap();
    let server = env::var("SERVER").expect("set the server variable");
    let username = env::var("USERNAME").expect("set the username variable");
    let password = env::var("PASSWORD").expect("set the password variable");
    let admin_user = env::var("ADMIN").expect("set the admin variable");
    let mut core = Core::new().unwrap();
    let hdl = core.handle();
    let mut mx = core.run(MatrixClient::login(&username, &password, &server, &hdl)).unwrap();
    println!("[+] Connected to {} as {}", server, username);
    let ss = mx.get_sync_stream();
    let hyper = Rc::new(RefCell::new(mx.get_hyper().clone()));
    let mx = Rc::new(RefCell::new(mx));
    let fut = ss.skip(1).for_each(|sync| {
        let mut futs: Vec<MatrixFuture<()>> = vec![];
        for (room, events) in sync.rooms.join {
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
                            let admin = meta.sender == admin_user;
                            let mx2 = mx.clone();
                            let rc = room.clone();
                            let fut = on_msg(mx.clone(), hyper.clone(), room.clone(), body, admin)
                                .or_else(move |err| {
                                    let mut mx = mx2.borrow_mut();
                                    let mut rc = rc.cli(&mut mx);
                                    rc.send_simple(format!("[error] {}", err))
                                        .map(|_| ())
                                });
                            futs.push(Box::new(fut));
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
