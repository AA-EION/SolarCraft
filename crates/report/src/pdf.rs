// SPDX-License-Identifier: GPL-3.0-or-later
//! PDF rendering with an embedded Typst compiler. The world is fully in
//! memory: template, data and charts are virtual files; fonts are bundled
//! (Source Sans 3, OFL). No file system or network access during rendering.
use crate::View;
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst_layout::PagedDocument;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

const TEMPLATE: &str = include_str!("../templates/customer.typ");

struct ReportWorld { library: LazyHash<Library>, book: LazyHash<FontBook>, fonts: Vec<Font>, main: FileId, files: Vec<(FileId, Bytes)>, today: Datetime }

fn id(path: &str) -> FileId { RootedPath::new(VirtualRoot::Project, VirtualPath::new(path).expect("valid virtual path")).intern() }

impl World for ReportWorld {
    fn library(&self) -> &LazyHash<Library> { &self.library }
    fn book(&self) -> &LazyHash<FontBook> { &self.book }
    fn main(&self) -> FileId { self.main }
    fn source(&self, fid: FileId) -> FileResult<Source> {
        if fid == self.main { return Ok(Source::new(fid, TEMPLATE.to_string())); }
        Err(FileError::NotSource)
    }
    fn file(&self, fid: FileId) -> FileResult<Bytes> {
        self.files.iter().find(|(f, _)| *f == fid).map(|(_, b)| b.clone()).ok_or_else(|| FileError::NotFound(fid.vpath().get_without_slash().into()))
    }
    fn font(&self, index: usize) -> Option<Font> { self.fonts.get(index).cloned() }
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> { Some(self.today) }
}

pub fn render(view: &View, date: (i32, u8, u8)) -> Result<Vec<u8>, String> {
    let fonts: Vec<Font> = crate::FONTS.iter().flat_map(|b| Font::iter(Bytes::new(b.to_vec()))).collect();
    let data = serde_json::to_vec(view).map_err(|e| e.to_string())?;
    let world = ReportWorld {
        library: LazyHash::new(Library::default()),
        book: LazyHash::new(FontBook::from_fonts(&fonts)),
        fonts,
        main: id("main.typ"),
        files: vec![
            (id("data.json"), Bytes::new(data)),
            (id("monthly.svg"), Bytes::new(view.monthly_svg.clone().into_bytes())),
            (id("waterfall.svg"), Bytes::new(view.waterfall_svg.clone().into_bytes())),
        ],
        today: Datetime::from_ymd(date.0, date.1, date.2).ok_or("invalid date")?,
    };
    let doc: PagedDocument = typst::compile(&world).output.map_err(|errs| errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; "))?;
    typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default()).map_err(|errs| errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; "))
}
