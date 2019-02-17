use self::super::{parse, ErroneousBodyPath, HrxError};
use jetscii::Substring as SubstringSearcher;
use linked_hash_map::LinkedHashMap;
use std::borrow::Borrow;
use std::str::FromStr;
use std::{iter, fmt};


/// A Human-Readable Archive, consisting of an optional comment and some entries, all separated by the boundary.
///
/// The archive boundary consists of a particular-length sequence of `=`s bounded with `<` and `>` on either side;
/// that sequence must be consistent across  the entirety of the archive, which means that no `body`
/// (be it a comment or file contents) can contain a newline followed by the boundary.
///
/// However, there is no way to enforce that on the typesystem level, meaning that the entries and comments can be modified at will,
/// so instead the archive will automatically check for boundary validity when
///
///   1. changing the global boundary length (via [`set_boundary_length()`](#method.set_boundary_length)) and
///   2. serialising to an output stream (be it via the `Display` impl, [`serialise()`](#method.serialise), or any derivatives thereof)
///
/// and return the path to the first erroneous (i.e. boundary-containing) `body`.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct HrxArchive {
    /// Some optional metadata.
    ///
    /// Cannot contain a newline followed by a boundary.
    pub comment: Option<String>,
    /// Some optional archive entries with their paths.
    pub entries: LinkedHashMap<HrxPath, HrxEntry>,

    pub(crate) boundary_length: usize,
}

/// A single entry in the archive, consisting of an optional comment and some data.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct HrxEntry {
    /// Some optional metadata.
    ///
    /// Cannot contain a newline followed by a boundary.
    pub comment: Option<String>,
    /// The specific entry data.
    pub data: HrxEntryData,
}

/// Some variant of an entry's contained data.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum HrxEntryData {
    /// File with some optional contents.
    ///
    /// Cannot contain a newline followed by a boundary.
    File { body: Option<String>, },
    /// Bodyless directory.
    Directory,
}

/// Verified-valid path to an entry in the archive.
///
/// Paths consist of `/`-separated components, each one consisting of characters higher than U+001F, except `/`, `\\` and `:`.
/// Components cannot be `.` nor `..`.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct HrxPath(pub(crate) String);


impl HrxArchive {
    /// Get the current boundary length, i.e. the amount of `=` characters in the boundary.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::str::FromStr;
    /// # use hrx::HrxArchive;
    /// let arch_str = r#"<===> input.scss
    /// ul {
    ///   margin-left: 1em;
    ///   li {
    ///     list-style-type: none;
    ///   }
    /// }
    ///
    /// <===> output.css
    /// ul {
    ///   margin-left: 1em;
    /// }
    /// ul li {
    ///   list-style-type: none;
    /// }"#;
    ///
    /// let arch = HrxArchive::from_str(arch_str).unwrap();
    /// assert_eq!(arch.boundary_length(), 3);
    /// ```
    pub fn boundary_length(&self) -> usize {
        self.boundary_length
    }

    /// Set new boundary length, if valid.
    ///
    /// Checks, whether any `body` within the archive contains the new boundary;
    /// if so – errors out with the path to the first one,
    /// otherwise sets the boundary length to the specified value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use hrx::{ErroneousBodyPath, HrxArchive};
    /// # use std::str::FromStr;
    /// let arch_str = r#"<===> boundary-5.txt
    /// This file contains a 5-length boundary:
    /// <=====>
    /// ^ right there
    ///
    /// <===>
    /// This is a comment,
    /// <=======>
    /// which contains a 7-length boundary.
    ///
    /// <===> fine.txt
    /// This file consists of
    /// multiple lines, but none of them
    /// starts with any sort of boundary-like string"#;
    ///
    /// let mut arch = HrxArchive::from_str(arch_str).unwrap();
    /// assert_eq!(arch.boundary_length(), 3);
    ///
    /// assert_eq!(arch.set_boundary_length(4), Ok(()));
    /// assert_eq!(arch.boundary_length(), 4);
    ///
    /// assert_eq!(arch.set_boundary_length(5),
    ///            Err(ErroneousBodyPath::EntryData("boundary-5.txt".to_string()).into()));
    /// assert_eq!(arch.boundary_length(), 4);
    ///
    /// assert_eq!(arch.set_boundary_length(6), Ok(()));
    /// assert_eq!(arch.boundary_length(), 6);
    ///
    /// assert_eq!(arch.set_boundary_length(7),
    ///            Err(ErroneousBodyPath::EntryComment("fine.txt".to_string()).into()));
    /// assert_eq!(arch.boundary_length(), 6);
    ///
    /// assert_eq!(arch.set_boundary_length(8), Ok(()));
    /// assert_eq!(arch.boundary_length(), 8);
    /// ```
    pub fn set_boundary_length(&mut self, new_len: usize) -> Result<(), HrxError> {
        self.validate_boundlen(new_len)?;
        self.boundary_length = new_len;
        Ok(())
    }

    /// Validate that no `body`s contain a `boundary` or error out with the path to the first one that does,
    ///
    /// # Examples
    ///
    /// ```
    /// # use hrx::{ErroneousBodyPath, HrxArchive};
    /// # use std::str::FromStr;
    /// let arch_str = r#"<===>
    /// A HRX file may consist of only a comment and nothing else."#;
    ///
    /// let mut arch = HrxArchive::from_str(arch_str).unwrap();
    /// assert_eq!(arch.validate_content(), Ok(()));
    ///
    /// *arch.comment.as_mut().unwrap() += "\n<===>\nYeehaw – now the comment contains the boundary!";
    /// assert_eq!(arch.validate_content(), Err(ErroneousBodyPath::RootComment.into()));
    /// ```
    pub fn validate_content(&self) -> Result<(), HrxError> {
        // TODO: make the test use new()
        self.validate_boundlen(self.boundary_length)
    }

    fn validate_boundlen(&self, len: usize) -> Result<(), HrxError> {
        // TODO: sanitise >0
        let bound: String = "\n<".chars().chain(iter::repeat('=').take(len)).chain(">".chars()).collect();
        let ss = SubstringSearcher::new(&bound);

        verify_opt(&self.comment, &ss).map_err(|_| ErroneousBodyPath::RootComment)?;
        for (pp, dt) in &self.entries {
            verify_opt(&dt.comment, &ss).map_err(|_| ErroneousBodyPath::EntryComment(pp.to_string()))?;
            match dt.data {
                HrxEntryData::File { ref body } => verify_opt(&body, &ss).map_err(|_| ErroneousBodyPath::EntryData(pp.to_string()))?,
                HrxEntryData::Directory => {}
            }
        }

        Ok(())
    }
}

fn verify_opt(which: &Option<String>, with: &SubstringSearcher) -> Result<(), ()> {
    if let Some(dt) = which.as_ref() {
        if with.find(dt).is_some() {
            return Err(());
        }
    }

    Ok(())
}

impl FromStr for HrxArchive {
    type Err = HrxError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let width = parse::discover_first_boundary_length(s).ok_or(HrxError::NoBoundary)?;
        let parsed = parse::archive(s, width)?;

        Ok(parsed)
    }
}

impl HrxPath {
    /// Unwraps the contained path.
    ///
    /// ```
    /// # use hrx::HrxPath;
    /// # use std::str::FromStr;
    /// let path = HrxPath::from_str("хэнло/communism.exe").unwrap();
    /// let raw = path.into_inner();
    ///
    /// assert_eq!(raw, "хэнло/communism.exe");
    /// ```
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for HrxPath {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&self.0)
    }
}

impl FromStr for HrxPath {
    type Err = HrxError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = parse::path(s, 0)?;

        Ok(parsed)
    }
}

impl Borrow<str> for HrxPath {
    fn borrow(&self) -> &str {
        &self.0
    }
}
