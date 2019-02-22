use self::super::{parse, ErroneousBodyPath, HrxError};
use jetscii::Substring as SubstringSearcher;
use self::super::output::write_archive;
use std::io::{Error as IoError, Write};
use self::super::util::boundary_str;
use linked_hash_map::LinkedHashMap;
use std::num::NonZeroUsize;
use std::borrow::Borrow;
use std::str::FromStr;
use std::fmt;


/// A Human-Readable Archive, consisting of an optional comment and some entries, all separated by the boundary.
///
/// The archive boundary consists of a particular-length sequence of `=`s bounded with `<` and `>` on either side;
/// that sequence must be consistent across  the entirety of the archive, which means that no `body`
/// (be it a comment or file contents) can contain a newline followed by the boundary.
///
/// However, there is no way to enforce that on the typesystem level,
/// meaning that the entries and comments can be modified at will,
/// so instead the archive will automatically check for boundary validity when
///
///   1. changing the global boundary length (via [`set_boundary_length()`](#method.set_boundary_length)) and
///   2. serialising to an output stream (usually via [`serialise()`](#method.serialise))
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

    pub(crate) boundary_length: NonZeroUsize,
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
    /// Create an empty archive with the specified boundary length.
    pub fn new(boundary_length: NonZeroUsize) -> HrxArchive {
        HrxArchive {
            comment: None,
            entries: LinkedHashMap::new(),
            boundary_length: boundary_length,
        }
    }

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
    /// assert_eq!(arch.boundary_length().get(), 3);
    /// ```
    pub fn boundary_length(&self) -> NonZeroUsize {
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
    /// # use std::num::NonZeroUsize;
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
    /// assert_eq!(arch.boundary_length().get(), 3);
    ///
    /// assert_eq!(arch.set_boundary_length(NonZeroUsize::new(4).unwrap()), Ok(()));
    /// assert_eq!(arch.boundary_length().get(), 4);
    ///
    /// assert_eq!(arch.set_boundary_length(NonZeroUsize::new(5).unwrap()),
    ///            Err(ErroneousBodyPath::EntryData("boundary-5.txt".to_string()).into()));
    /// assert_eq!(arch.boundary_length().get(), 4);
    ///
    /// assert_eq!(arch.set_boundary_length(NonZeroUsize::new(6).unwrap()), Ok(()));
    /// assert_eq!(arch.boundary_length().get(), 6);
    ///
    /// assert_eq!(arch.set_boundary_length(NonZeroUsize::new(7).unwrap()),
    ///            Err(ErroneousBodyPath::EntryComment("fine.txt".to_string()).into()));
    /// assert_eq!(arch.boundary_length().get(), 6);
    ///
    /// assert_eq!(arch.set_boundary_length(NonZeroUsize::new(8).unwrap()), Ok(()));
    /// assert_eq!(arch.boundary_length().get(), 8);
    /// ```
    pub fn set_boundary_length(&mut self, new_len: NonZeroUsize) -> Result<(), HrxError> {
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
    /// # use std::num::NonZeroUsize;
    /// let mut arch = HrxArchive::new(NonZeroUsize::new(3).unwrap());
    /// arch.comment = Some("Yeehaw! the comment\n<===>\n contains the boundary!".to_string());
    ///
    /// assert_eq!(arch.validate_content(), Err(ErroneousBodyPath::RootComment.into()));
    /// ```
    pub fn validate_content(&self) -> Result<(), HrxError> {
        self.validate_boundlen(self.boundary_length)
    }

    fn validate_boundlen(&self, len: NonZeroUsize) -> Result<(), HrxError> {
        let bound = boundary_str(len);
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

    /// Write the archive out to the specified output stream, after verification.
    ///
    /// The compound result type is due to the fact that `std::io::Error` doesn't play well with having it in an enum variant.
    ///
    /// # Examples
    ///
    /// Failed validation:
    ///
    /// ```
    /// # use hrx::{ErroneousBodyPath, HrxArchive, HrxPath};
    /// # use std::num::NonZeroUsize;
    /// let mut arch = HrxArchive::new(NonZeroUsize::new(3).unwrap());
    /// arch.comment = Some("Yeehaw! the comment\n<===>\n contains the boundary!".to_string());
    ///
    /// let mut out = vec![];
    /// assert_eq!(arch.serialise(&mut out).unwrap_err().unwrap(),
    ///            ErroneousBodyPath::RootComment.into());
    /// // Note how the returned result cannot be directly compared to,
    /// // as a byproduct of `std::io::Error` being contained therein.
    /// ```
    ///
    /// Generation:
    ///
    /// ```
    /// # use hrx::{ErroneousBodyPath, HrxEntryData, HrxArchive, HrxEntry, HrxPath};
    /// # use std::num::NonZeroUsize;
    /// let mut arch = HrxArchive::new(NonZeroUsize::new(5).unwrap());
    /// arch.comment =
    ///     Some("This is the archive comment, forthlaying its contents' description".to_string());
    ///
    /// arch.entries.insert("directory".parse().unwrap(), HrxEntry {
    ///     comment: Some("This directory contains files!".to_string()),
    ///     data: HrxEntryData::Directory,
    /// });
    ///
    /// arch.entries.insert("directory/dsc.txt".parse().unwrap(), HrxEntry {
    ///     comment:
    ///         Some("This file forthlays the building blocks of any stable society".to_string()),
    ///     data: HrxEntryData::File {
    ///         body: Some("Коммунизм!\n".to_string()),
    ///     },
    /// });
    ///
    /// let mut out = vec![];
    /// arch.serialise(&mut out).unwrap();
    /// assert_eq!(String::from_utf8(out).unwrap(), r#"<=====>
    /// This directory contains files!
    /// <=====> directory/
    /// <=====>
    /// This file forthlays the building blocks of any stable society
    /// <=====> directory/dsc.txt
    /// Коммунизм!
    ///
    /// <=====>
    /// This is the archive comment, forthlaying its contents' description"#);
    /// ```
    ///
    /// Transserialisation:
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
    ///
    /// let mut out = vec![];
    /// arch.serialise(&mut out).unwrap();
    /// assert_eq!(String::from_utf8(out).unwrap(), arch_str);
    /// ```
    pub fn serialise<W: Write>(&self, into: &mut W) -> Result<(), Result<HrxError, IoError>> {
        write_archive(&self, into)
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
        let parsed = parse::path(s, NonZeroUsize::new(1).unwrap())?;

        Ok(parsed)
    }
}

impl Borrow<str> for HrxPath {
    fn borrow(&self) -> &str {
        &self.0
    }
}
