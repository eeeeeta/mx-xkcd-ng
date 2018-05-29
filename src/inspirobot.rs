use futures::prelude::*;
use hyper::Uri;
use gm::room::{Room, RoomExt};
use gm::types::messages::{Message, ImageInfo};
use image::{self, GenericImage};
use gm::media::Media;
use super::{Hyper, Mx, validate_response};

pub fn inspirobot(h: Hyper, mut cli: Mx, r: Room<'static>) -> impl Future<Item = (), Error = ::failure::Error> {
    async_block! {
        let uri: Uri = "http://inspirobot.me/api?generate=true".parse()?;
        let resp = await!(h.borrow_mut().get(uri))?;
        validate_response(&resp)?;
        let body = await!(resp.body().concat2())?;
        let uri: Uri = String::from_utf8(body.to_vec())?.parse()?;
        let resp = await!(h.borrow_mut().get(uri))?;
        validate_response(&resp)?;
        let body = await!(resp.body().concat2())?;
        let img = image::load_from_memory_with_format(
            &body as &[u8],
            image::ImageFormat::JPEG
            )?;
        let info = ImageInfo {
            mimetype: "image/jpeg".into(),
            h: img.height(),
            w: img.width(),
            size: body.len() as _
        };
        let data: Vec<u8> = body.into_iter().collect();
        let rpl = await!(Media::upload(&mut cli, data, &info.mimetype))?;
        let img = Message::Image {
            body: "inspirobot".into(),
            info: Some(info),
            thumbnail_info: None,
            thumbnail_url: None,
            url: rpl.content_uri
        };
        await!(r.cli(&mut cli).send(img))?;
        Ok(())
    }
}
