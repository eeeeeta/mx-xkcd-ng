use futures::prelude::*;
use serde_json;
use image::{self, GenericImage};
use hyper::Uri;
use gm::room::{Room, RoomExt};
use gm::types::messages::{Message, ImageInfo};
use gm::media::Media;
use super::{Hyper, Mx, validate_response};

#[derive(Deserialize)]
pub struct Comic {
    num: u32,
    img: String,
    #[serde(skip_deserializing)]
    data: Option<Vec<u8>>,
    alt: String,
    title: String
}
pub fn fetch_comic(h: Hyper, n: Option<u32>) -> impl Future<Item = Comic, Error = ::failure::Error> {
    async_block! {
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
        c.data = Some(body.into_iter().collect());
        Ok(c)
    }
}
pub fn send_comic(mut cli: Mx, r: Room<'static>, mut c: Comic) -> impl Future<Item = (), Error = ::failure::Error> {
    async_block! {
        let data = c.data.take().unwrap();
        let img = image::load_from_memory(&data as &[u8])?;
        let info = ImageInfo {
            mimetype: "image/png".into(),
            h: img.height(),
            w: img.width(),
            size: data.len() as _
        };
        let rpl = await!(Media::upload(&mut cli, data, &info.mimetype))?;
        await!(r.cli(&mut cli)
               .send_simple(format!("{}: {}", c.num, c.title)))?;
        let img = Message::Image {
            body: c.title.clone(),
            info: Some(info),
            thumbnail_info: None,
            thumbnail_url: None,
            url: rpl.content_uri
        };
        await!(r.cli(&mut cli)
               .send(img))?;
        await!(r.cli(&mut cli)
               .send_simple(format!("Alt text: {}", c.alt)))?;
        Ok(())
    }
}
