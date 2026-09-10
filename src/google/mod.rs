pub mod client;
pub mod groups;

use std::error::Error;

pub type GoogleError = Box<dyn Error + Send + Sync>;
