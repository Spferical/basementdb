#![feature(plugin, use_extern_macros)]
#![plugin(tarpc_plugins)]

extern crate serde;

#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate tarpc;
extern crate tokio_core;

pub mod message;
pub mod signed;
pub mod zeno;
