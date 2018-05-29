use futures::prelude::*;
use gm::room::{Room, RoomExt};
use super::Mx;
use gm::errors::MatrixError;

#[derive(Serialize, Deserialize, Default)]
pub struct Lolcount {
    lol_count: u32
}

pub fn get_lols(mut cli: Mx, r: Room<'static>) -> impl Future<Item = u32, Error = ::failure::Error> {
    async_block! {
        let res = await!(r.cli(&mut cli).get_typed_state::<Lolcount>("org.eu.theta.lolcount", None));
        match res {
            Ok(l) => Ok(l.lol_count),
            Err(e) => {
                if let MatrixError::BadRequest(ref brk) = e {
                    if brk.errcode == "M_NOT_FOUND" {
                        return Ok(0)
                    }
                }
                Err(e.into())
            }
        }
    }
}
pub fn set_lols(mut cli: Mx, r: Room<'static>, lc: u32) -> impl Future<Item = (), Error = ::failure::Error> {
    async_block! {
        let lolcount = Lolcount { lol_count: lc };
        await!(r.cli(&mut cli).set_typed_state("org.eu.theta.lolcount", None, lolcount))?;
        Ok(())
    }
}
