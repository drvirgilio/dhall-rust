/// The beginning of a file path which anchors subsequent path components
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum FilePrefix {
    /// Absolute path
    Absolute,
    /// Path relative to .
    Here,
    /// Path relative to ..
    Parent,
    /// Path relative to ~
    Home,
}

/// The location of import (i.e. local vs. remote vs. environment)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportLocation {
    Local(FilePrefix, Vec<String>),
    Remote(URL),
    Env(String),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct URL {
    pub scheme: Scheme,
    pub authority: String,
    pub path: Vec<String>,
    pub query: Option<String>,
    pub headers: Option<Box<ImportHashed>>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Scheme {
    HTTP,
    HTTPS,
}

/// How to interpret the import's contents (i.e. as Dhall code or raw text)
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ImportMode {
    Code,
    RawText,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hash {
    pub protocol: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportHashed {
    pub location: ImportLocation,
    pub hash: Option<Hash>,
}

/// Reference to an external resource
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Import {
    pub mode: ImportMode,
    pub location_hashed: ImportHashed,
}
