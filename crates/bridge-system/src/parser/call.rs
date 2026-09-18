//! The call-token grammar (winnow).
//!
//! ```ebnf
//! calltok    = "(" callcore ")" | callcore ;
//! callcore   = "P" | "D" | "R" | "X" | "XX"
//!            | level strainspec
//!            | level ( "step" | "steps" )
//!            | callcore "/" ( callcore | strainspec ) ;      (* 2S/3H, 4D/H *)
//! level      = "1".."7" | "n" ;
//! strainspec = literal | variable | "red" | "black" ;
//! literal    = "NT" | "N" | ( "C" | "D" | "H" | "S" ) { "C" | "D" | "H" | "S" } ;
//! variable   = "M" | "m" | "oM" | "om" | "X" | "Y" | "Z" | "x" | "y" | "z" ;
//! ```

use winnow::prelude::*;

use crate::pattern::{CallPattern, Side};

/// Parses one call token (with optional parentheses for the opponents' calls).
pub fn calltok(input: &mut &str) -> ModalResult<(Side, CallPattern)> {
    todo!("phase 3")
}

/// Parses a history row: `1N-2C;`, `(1NT)---`, `1C-(1D)-`.
pub fn history(input: &mut &str) -> ModalResult<Vec<(Side, CallPattern)>> {
    todo!("phase 3")
}
