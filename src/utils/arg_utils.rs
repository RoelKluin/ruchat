use anyhow::Result;
use std::error::Error;

pub(super) fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<dyn Error + Send + Sync + 'static>>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
    U: std::str::FromStr,
    U::Err: Error + Send + Sync + 'static,
{
    match s.split_once(':') {
        Some((key, value)) => Ok((key.parse()?, value.parse()?)),
        None => Err(format!("invalid KEY:VALUE, no `:` found in `{}`", s).into()),
    }
}
