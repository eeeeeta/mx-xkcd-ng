use futures::prelude::*;
use gm::room::{Room, RoomExt};
use gm::errors::*;
use super::Mx;

#[derive(Serialize, Deserialize, Default)]
pub struct Lolcount {
    lol_count: u32
}

#[async]
pub fn get_lols(cli: Mx, r: Room<'static>) -> MatrixResult<u32> {
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
pub fn set_lols(cli: Mx, r: Room<'static>, lc: u32) -> MatrixResult<()> {
    let lolcount = Lolcount { lol_count: lc };
    await!(r.cli(&mut cli.borrow_mut())
           .set_state("org.eu.theta.lolcount", None, lolcount))?;
    Ok(())
}
