pub mod client;

use std::error::Error;
pub type GoogleError = Box<dyn Error + Send + Sync>;
