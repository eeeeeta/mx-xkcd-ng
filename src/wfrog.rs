use futures::prelude::*;
use gm::room::{Room, RoomExt};
use gm::errors::*;
use tokio_timer::Timer;
use std::time::Duration;
use tokio_core::reactor::Handle;
use std::rc::Rc;
use std::cell::RefCell;
use gm::types::messages::{Message, ImageInfo};
use chrono::prelude::*;
use super::Mx;

#[derive(Serialize, Deserialize)]
pub struct WfrogSettings {
    link: String,
    text: String,
    info: ImageInfo,
    enabled: bool
}
#[derive(Serialize, Deserialize)]
pub struct WfrogState {
    last: DateTime<Utc>
}
type WfrogRooms = Rc<RefCell<Vec<Room<'static>>>>;
#[async]
pub fn wfrog_set_text(mx: Mx, room: Room<'static>, text: String) -> MatrixResult<()> {
    let mut settings = await!(room.cli(&mut mx.borrow_mut())
                              .get_state::<WfrogSettings>("org.eu.theta.wfrog.settings", None))?;
    settings.text = text;
    await!(room.cli(&mut mx.borrow_mut())
           .set_state("org.eu.theta.wfrog.settings", None, settings))?;
    await!(room.cli(&mut mx.borrow_mut())
           .send_simple("Done."))?;
    Ok(())
}
#[async]
pub fn wfrog_disable_enable(mx: Mx, room: Room<'static>, which: bool) -> MatrixResult<()> {
    let mut settings = await!(room.cli(&mut mx.borrow_mut())
                              .get_state::<WfrogSettings>("org.eu.theta.wfrog.settings", None))?;
    settings.enabled = which;
    await!(room.cli(&mut mx.borrow_mut())
           .set_state("org.eu.theta.wfrog.settings", None, settings))?;
    await!(room.cli(&mut mx.borrow_mut())
           .send_simple("Done."))?;
    Ok(())
}
#[async]
pub fn wfrog_explain_state(mx: Mx, room: Room<'static>) -> MatrixResult<()> {
    let state = await!(room.cli(&mut mx.borrow_mut())
                     .get_state::<WfrogState>("org.eu.theta.wfrog.state", None));
    let settings = await!(room.cli(&mut mx.borrow_mut())
                          .get_state::<WfrogSettings>("org.eu.theta.wfrog.settings", None));
    if let Ok(s) = settings {
        let state = if s.enabled {
            "Wednesday frog is active!"
        }
        else {
            "Wednesday frog is currently disabled."
        };
        let text = if s.text != "" {
            format!("The text configured is: {}", s.text)
        } else {
            "(Configure text with the command 'mittwoch text [text goes here]'.)".into()
        };
        let st = format!("{}\n{}\nThe image configured is:", state, text);
        await!(room.cli(&mut mx.borrow_mut())
               .send_simple(st))?;
        let img = Message::Image {
            body: s.text,
            info: Some(s.info),
            thumbnail_info: None,
            thumbnail_url: None,
            url: s.link
        };
        await!(room.cli(&mut mx.borrow_mut())
               .send(img))?;
    }
    else {
        let st = r#"No Wednesday frog settings are active in this room (or fetching them failed)
To configure Wednesday frog, send an image with the filename "mittwoch.png".
You must have power level 20 or greater to do so.
"#;
        await!(room.cli(&mut mx.borrow_mut())
               .send_simple(st))?;
    }
    if let Ok(s) = state {
        await!(room.cli(&mut mx.borrow_mut())
               .send_simple(format!("Wednesday frog last triggered at: {}", s.last)))?;
    }
    Ok(())
}
#[async]
pub fn wfrog_set_image(mx: Mx, room: Room<'static>, url: String, info: ImageInfo) -> MatrixResult<()> {
    let settings = WfrogSettings {
        link: url,
        text: "".into(),
        info,
        enabled: true
    };
    await!(room.cli(&mut mx.borrow_mut())
           .set_state("org.eu.theta.wfrog.settings", None, settings))?;
    Ok(())
}
#[async]
pub fn wfrog_called(mx: Mx, rooms: WfrogRooms) -> MatrixResult<()> {
    let rooms = { rooms.borrow().clone() };
    for room in rooms.into_iter() {
        let rm = room.clone();
        if let Err(e) = await!(wfrog_process_room(mx.clone(), room)) {
            println!("failed to process wfrog for room {}: {}", rm.id, e);
        }
    }
    Ok(())
}
#[async]
pub fn wfrog_process_room(mx: Mx, room: Room<'static>) -> MatrixResult<()> {
    // check if we've already said stuff
    let res = await!(room.cli(&mut mx.borrow_mut())
                     .get_state::<WfrogState>("org.eu.theta.wfrog.state", None));
    let res = res.or_else(|e| {
        if let &MatrixErrorKind::BadRequest(ref brk) = e.kind() {
            if brk.errcode == "M_NOT_FOUND" {
                return Ok(WfrogState {
                    last: Utc.timestamp(0, 0)
                });
            }
        }
        Err(e)
    })?;
    let now = Utc::now();
    if res.last.ordinal() == now.ordinal() {
        // es ist Mittwoch, meine Kerle!
        let settings = await!(room.cli(&mut mx.borrow_mut())
                              .get_state::<WfrogSettings>("org.eu.theta.wfrog.settings", None));
        if let Err(ref e) = settings {
            if let &MatrixErrorKind::BadRequest(ref brk) = e.kind() {
                if brk.errcode == "M_NOT_FOUND" {
                    // Not configured for this room, don't send anything.
                    return Ok(());
                }
            }
        }
        let settings = settings?;
        if !settings.enabled {
            return Ok(());
        }
        if settings.text != "" {
            await!(room.cli(&mut mx.borrow_mut())
                   .send_simple(settings.text.clone()))?;
        }
        let img = Message::Image {
            body: settings.text,
            info: Some(settings.info),
            thumbnail_info: None,
            thumbnail_url: None,
            url: settings.link
        };
        await!(room.cli(&mut mx.borrow_mut())
               .send(img))?;
        let state = WfrogState { last: now };
        await!(room.cli(&mut mx.borrow_mut())
               .set_state("org.eu.theta.wfrog.state", None, state))?;
    }
    Ok(())
}
pub fn wfrog_init(mx: Mx, hdl: &Handle, rooms: WfrogRooms) {
    let timer = Timer::default();
    let inter = timer.interval(Duration::from_millis(60_000));
    let fut = inter.map_err(|_| ()).for_each(move |_| {
        wfrog_called(mx.clone(), rooms.clone())
            .map_err(|_| ())
    });
    hdl.spawn(fut);
}
