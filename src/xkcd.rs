use futures::prelude::*;
use serde_json;
use image::{self, GenericImage};
use hyper::{Chunk, Uri};
use gm::errors::*;
use gm::room::{Room, RoomExt};
use gm::types::messages::{Message, ImageInfo};
use gm::http::ContentType;
use super::{Hyper, Mx, validate_response};

#[derive(Deserialize)]
pub struct Comic {
    num: u32,
    img: String,
    #[serde(skip_deserializing)]
    data: Option<Chunk>,
    alt: String,
    title: String
}
#[async]
pub fn fetch_comic(h: Hyper, n: Option<u32>) -> MatrixResult<Comic> {
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
pub fn send_comic(cli: Mx, r: Room<'static>, mut c: Comic) -> MatrixResult<()> {
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
