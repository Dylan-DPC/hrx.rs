//! This is an adaptation of the example/invalid/duplicates.hrx file in the original google/hrx repository.


use hrx::{HrxEntryData, HrxArchive, HrxEntry, HrxError};
use std::str::FromStr;


#[test]
fn duplicate_files() {
    assert_eq!(HrxArchive::from_str("<======> file\n<======> file\n"),
               Err(HrxError::DuplicateEntry("file".to_string(),
                                            HrxEntry {
                                                comment: None,
                                                data: HrxEntryData::File { body: None },
                                            },
                                            HrxEntry {
                                                comment: None,
                                                data: HrxEntryData::File { body: None },
                                            })));
}

#[test]
fn duplicate_dirs() {
    assert_eq!(HrxArchive::from_str("<======> dir/\n<======> dir/\n"),
               Err(HrxError::DuplicateEntry("dir".to_string(),
                                            HrxEntry {
                                                comment: None,
                                                data: HrxEntryData::Directory,
                                            },
                                            HrxEntry {
                                                comment: None,
                                                data: HrxEntryData::Directory,
                                            })));
}

#[test]
fn file_as_parent() {
    assert_eq!(HrxArchive::from_str("<======> file\n<======> file/sub\n"),
               Err(HrxError::FileAsDirectory("file".to_string(), "file/sub".to_string())));
}
