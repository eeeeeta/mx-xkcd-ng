use futures::prelude::*;
use hyper::Uri;
use gm::http::ContentType;
use gm::room::{Room, RoomExt};
use gm::types::messages::{Message, ImageInfo};
use gm::errors::*;
use image::{self, GenericImage};
use super::{Hyper, Mx, validate_response};

#[async]
pub fn inspirobot(h: Hyper, cli: Mx, r: Room<'static>) -> MatrixResult<()> {
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
