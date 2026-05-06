use std::{path::Path, time::SystemTime};

use async_trait::async_trait;
use serde::Deserialize;

use crate::{Dirent, DirSource, FileType};

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const DOC_MIME: &str = "application/vnd.google-apps.document";
const SHEET_MIME: &str = "application/vnd.google-apps.spreadsheet";
const SLIDES_MIME: &str = "application/vnd.google-apps.presentation";

const FIELDS_FILE: &str = "id,name,mimeType,size,createdTime,modifiedTime";

pub struct GoogleDriveDirSource {
    client: reqwest::Client,
    access_token: String,
    root_folder_id: String,
}

#[derive(Debug, Deserialize)]
struct DriveFile {
    id: String,
    name: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DriveListResp {
    files: Vec<DriveFile>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

impl GoogleDriveDirSource {
    pub fn new(access_token: impl Into<String>, root_folder_id: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            access_token: access_token.into(),
            root_folder_id: root_folder_id.into(),
        }
    }

    async fn get_metadata(&self, id: &str) -> Option<DriveFile> {
        let url = format!("https://www.googleapis.com/drive/v3/files/{}", id);
        self.client
            .get(&url)
            .bearer_auth(&self.access_token)
            .query(&[("fields", FIELDS_FILE)])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json::<DriveFile>()
            .await
            .ok()
    }

    async fn list_query(&self, q: &str, page_token: Option<&str>) -> Option<DriveListResp> {
        let fields = format!("nextPageToken,files({})", FIELDS_FILE);
        let mut req = self
            .client
            .get("https://www.googleapis.com/drive/v3/files")
            .bearer_auth(&self.access_token)
            .query(&[("q", q), ("fields", &fields), ("pageSize", "1000")]);
        if let Some(t) = page_token {
            req = req.query(&[("pageToken", t)]);
        }
        req.send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json::<DriveListResp>()
            .await
            .ok()
    }

    async fn list_children(&self, folder_id: &str) -> Vec<DriveFile> {
        let q = format!("'{}' in parents and trashed = false", folder_id);
        let mut all = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let resp = match self.list_query(&q, token.as_deref()).await {
                Some(r) => r,
                None => break,
            };
            all.extend(resp.files);
            match resp.next_page_token {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        all
    }

    async fn resolve(&self, path: &Path) -> Option<DriveFile> {
        let mut current = self.get_metadata(&self.root_folder_id).await?;
        for comp in path.components() {
            use std::path::Component;
            let name = match comp {
                Component::Normal(s) => s.to_string_lossy().into_owned(),
                Component::RootDir | Component::CurDir => continue,
                Component::ParentDir | Component::Prefix(_) => return None,
            };
            if current.mime_type != FOLDER_MIME {
                return None;
            }
            let q = format!(
                "'{}' in parents and name = '{}' and trashed = false",
                current.id,
                name.replace('\\', "\\\\").replace('\'', "\\'"),
            );
            let resp = self.list_query(&q, None).await?;
            current = resp.files.into_iter().next()?;
        }
        Some(current)
    }

    async fn build_dirent(&self, file: &DriveFile) -> Dirent {
        let created_at = SystemTime::UNIX_EPOCH;
        let modified_at = SystemTime::UNIX_EPOCH;
        if file.mime_type == FOLDER_MIME {
            let mut children = Vec::new();
            for child in self.list_children(&file.id).await {
                children.push(Box::pin(self.build_dirent(&child)).await);
            }
            Dirent::Dir {
                name: file.name.clone(),
                children,
                created_at,
                modified_at,
            }
        } else {
            let sz = file
                .size
                .as_deref()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            Dirent::File {
                name: file.name.clone(),
                ftype: ftype_for(&file.mime_type, &file.name),
                sz,
                created_at,
                modified_at,
            }
        }
    }
}

#[async_trait]
impl DirSource for GoogleDriveDirSource {
    async fn list(&self, path: &Path) -> Option<Dirent> {
        let file = self.resolve(path).await?;
        Some(self.build_dirent(&file).await)
    }

    async fn read(&self, path: &Path) -> Option<Vec<u8>> {
        let file = self.resolve(path).await?;
        if file.mime_type == FOLDER_MIME {
            return None;
        }

        let req = if let Some(export_mime) = export_target(&file.mime_type) {
            self.client
                .get(format!(
                    "https://www.googleapis.com/drive/v3/files/{}/export",
                    file.id,
                ))
                .query(&[("mimeType", export_mime)])
        } else {
            self.client
                .get(format!(
                    "https://www.googleapis.com/drive/v3/files/{}",
                    file.id,
                ))
                .query(&[("alt", "media")])
        };

        req.bearer_auth(&self.access_token)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .bytes()
            .await
            .ok()
            .map(|b| b.to_vec())
    }
}

fn export_target(mime: &str) -> Option<&'static str> {
    match mime {
        DOC_MIME => Some("text/markdown"),
        SHEET_MIME => Some("text/csv"),
        SLIDES_MIME => Some("application/pdf"),
        _ => None,
    }
}

fn ftype_for(mime: &str, name: &str) -> FileType {
    match mime {
        DOC_MIME => FileType::Markdown,
        SHEET_MIME => FileType::Text,
        SLIDES_MIME => FileType::PDF,
        "application/pdf" => FileType::PDF,
        "text/markdown" => FileType::Markdown,
        "text/plain" => FileType::Text,
        _ => match Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("pdf") => FileType::PDF,
            Some("md" | "markdown") => FileType::Markdown,
            Some("txt") => FileType::Text,
            _ => FileType::Raw,
        },
    }
}
