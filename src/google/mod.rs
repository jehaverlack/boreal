pub mod auth;
pub mod bridge;
pub mod client;
pub mod groups;
pub mod my_groups;

use std::error::Error;

pub type GoogleError = Box<dyn Error + Send + Sync>;
