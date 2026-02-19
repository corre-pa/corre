use askama::Template;
use corre_core::publish::Edition;

#[derive(Template)]
#[template(path = "newspaper.html")]
pub struct NewspaperTemplate<'a> {
    pub title: &'a str,
    pub edition: &'a Edition,
}

#[derive(Template)]
#[template(path = "edition.html")]
pub struct EditionTemplate<'a> {
    pub title: &'a str,
    pub edition: &'a Edition,
    pub dates: &'a [chrono::NaiveDate],
}
