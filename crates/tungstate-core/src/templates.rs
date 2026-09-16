//! Starting layouts, compiled in.
//!
//! A folder with no rules is the wall between somebody and this half of the
//! product, because the only way past it was to hand-write TOML. These are the
//! way past: pick one, and it is written into the folder.
//!
//! They are *starting* files. The window never edits a policy — that decision
//! was made in slice 5b and holds — so after one of these is written, changing
//! it means opening it in an editor. Writing a first draft is scaffolding.

/// One layout somebody can start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Template {
    /// The token `init --template` takes.
    pub name: &'static str,
    /// What it does, in one line, for a list somebody is choosing from.
    pub summary: &'static str,
    /// Who it is for, in one more line.
    pub detail: &'static str,
    /// The file itself.
    pub body: &'static str,
}

/// Every layout, in the order a picker should show them.
pub const TEMPLATES: &[Template] = &[
    Template {
        name: "downloads",
        summary: "Sort by what each file is",
        detail: "Images, video, documents, archives and installers each get a \
                 directory. Reads the first few bytes, so a mislabelled file \
                 still lands in the right place.",
        body: include_str!("../../../policies/downloads.toml"),
    },
    Template {
        name: "photos",
        summary: "File photos by date",
        detail: "Year and month directories, using the camera's own date where \
                 the photo has one and the file's date where it does not.",
        body: include_str!("../../../policies/photos.toml"),
    },
    Template {
        name: "documents",
        summary: "Group documents by kind, then year",
        detail: "PDFs, writing and spreadsheets, each split by year. Reads \
                 nothing inside your files, so it is fast however large the \
                 folder is.",
        body: include_str!("../../../policies/documents.toml"),
    },
    Template {
        name: "by-type",
        summary: "One directory per file extension",
        detail: "The simplest rule there is, and a good way to see what is \
                 actually in a folder before deciding how you want it arranged.",
        body: include_str!("../../../policies/by-type.toml"),
    },
];

/// One layout by name.
#[must_use]
pub fn template(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// Every layout's name, for an error message that lists the choices.
#[must_use]
pub fn names() -> Vec<&'static str> {
    TEMPLATES.iter().map(|t| t.name).collect()
}
