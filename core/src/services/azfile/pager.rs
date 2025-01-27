// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::sync::Arc;

use async_trait::async_trait;
use http::StatusCode;
use quick_xml::de::from_str;
use serde::Deserialize;

use crate::raw::*;
use crate::*;

use super::core::AzfileCore;
use super::error::parse_error;

pub struct AzfilePager {
    core: Arc<AzfileCore>,
    path: String,
    limit: Option<usize>,
    done: bool,
    continuation: String,
}

impl AzfilePager {
    pub fn new(core: Arc<AzfileCore>, path: String, limit: Option<usize>) -> Self {
        Self {
            core,
            path,
            limit,
            done: false,
            continuation: "".to_string(),
        }
    }
}

#[async_trait]
impl oio::Page for AzfilePager {
    async fn next(&mut self) -> Result<Option<Vec<oio::Entry>>> {
        if self.done {
            return Ok(None);
        }

        let resp = self
            .core
            .azfile_list(&self.path, &self.limit, &self.continuation)
            .await?;

        let status = resp.status();

        if status != StatusCode::OK {
            if status == StatusCode::NOT_FOUND {
                return Ok(None);
            }
            return Err(parse_error(resp).await?);
        }

        let bs = resp.into_body().bytes().await?;

        let text = String::from_utf8(bs.to_vec()).expect("response convert to string must success");

        let results: EnumerationResults = from_str(&text).map_err(|e| {
            Error::new(ErrorKind::Unexpected, "deserialize xml from response").set_source(e)
        })?;

        let mut entries = Vec::new();

        for file in results.entries.file {
            let meta = Metadata::new(EntryMode::FILE)
                .with_etag(file.properties.etag)
                .with_content_length(file.properties.content_length.unwrap_or(0))
                .with_last_modified(parse_datetime_from_rfc2822(&file.properties.last_modified)?);
            let path = self.path.clone().trim_start_matches('/').to_string() + &file.name;
            entries.push(oio::Entry::new(&path, meta));
        }

        for dir in results.entries.directory {
            let meta = Metadata::new(EntryMode::DIR)
                .with_etag(dir.properties.etag)
                .with_last_modified(parse_datetime_from_rfc2822(&dir.properties.last_modified)?);
            let path = self.path.clone().trim_start_matches('/').to_string() + &dir.name + "/";
            entries.push(oio::Entry::new(&path, meta));
        }

        if results.next_marker.is_empty() {
            self.done = true;
        } else {
            self.continuation = results.next_marker;
        }

        if entries.is_empty() {
            Ok(None)
        } else {
            Ok(Some(entries))
        }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
struct EnumerationResults {
    marker: Option<String>,
    prefix: Option<String>,
    max_results: Option<u32>,
    directory_id: Option<String>,
    entries: Entries,
    #[serde(default)]
    next_marker: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
struct Entries {
    #[serde(default)]
    file: Vec<File>,
    #[serde(default)]
    directory: Vec<Directory>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
struct File {
    #[serde(rename = "FileId")]
    file_id: String,
    name: String,
    properties: Properties,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
struct Directory {
    #[serde(rename = "FileId")]
    file_id: String,
    name: String,
    properties: Properties,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
struct Properties {
    #[serde(rename = "Content-Length")]
    content_length: Option<u64>,
    #[serde(rename = "CreationTime")]
    creation_time: String,
    #[serde(rename = "LastAccessTime")]
    last_access_time: String,
    #[serde(rename = "LastWriteTime")]
    last_write_time: String,
    #[serde(rename = "ChangeTime")]
    change_time: String,
    #[serde(rename = "Last-Modified")]
    last_modified: String,
    #[serde(rename = "Etag")]
    etag: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_list_result() {
        let xml = r#"
<?xml version="1.0" encoding="utf-8"?>
<EnumerationResults ServiceEndpoint="https://myaccount.file.core.windows.net/" ShareName="myshare" ShareSnapshot="date-time" DirectoryPath="directory-path">
  <Marker>string-value</Marker>
  <Prefix>string-value</Prefix>
  <MaxResults>100</MaxResults>
  <DirectoryId>directory-id</DirectoryId>
  <Entries>
     <File>
        <Name>Rust By Example.pdf</Name>
        <FileId>13835093239654252544</FileId>
        <Properties>
            <Content-Length>5832374</Content-Length>
            <CreationTime>2023-09-25T12:43:05.8483527Z</CreationTime>
            <LastAccessTime>2023-09-25T12:43:05.8483527Z</LastAccessTime>
            <LastWriteTime>2023-09-25T12:43:08.6337775Z</LastWriteTime>
            <ChangeTime>2023-09-25T12:43:08.6337775Z</ChangeTime>
            <Last-Modified>Mon, 25 Sep 2023 12:43:08 GMT</Last-Modified>
            <Etag>\"0x8DBBDC4F8AC4AEF\"</Etag>
        </Properties>
    </File>
    <Directory>
        <Name>test_list_rich_dir</Name>
        <FileId>12105702186650959872</FileId>
        <Properties>
            <CreationTime>2023-10-15T12:03:40.7194774Z</CreationTime>
            <LastAccessTime>2023-10-15T12:03:40.7194774Z</LastAccessTime>
            <LastWriteTime>2023-10-15T12:03:40.7194774Z</LastWriteTime>
            <ChangeTime>2023-10-15T12:03:40.7194774Z</ChangeTime>
            <Last-Modified>Sun, 15 Oct 2023 12:03:40 GMT</Last-Modified>
            <Etag>\"0x8DBCD76C58C3E96\"</Etag>
        </Properties>
    </Directory>
  </Entries>
  <NextMarker />
</EnumerationResults>
        "#;

        let results: EnumerationResults = from_str(xml).unwrap();

        assert_eq!(results.entries.file[0].name, "Rust By Example.pdf");

        assert_eq!(
            results.entries.file[0].properties.etag,
            "\\\"0x8DBBDC4F8AC4AEF\\\""
        );

        assert_eq!(results.entries.directory[0].name, "test_list_rich_dir");

        assert_eq!(
            results.entries.directory[0].properties.etag,
            "\\\"0x8DBCD76C58C3E96\\\""
        );
    }
}
