//! Firefox Sync API - reserved for future cloud sync feature

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tracing::{debug, info};

/// Firefox Accounts配置
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirefoxAccountData {
    email: String,
    session_token: String,
    uid: String,
    verified: bool,
    oauth_tokens: OAuthTokens,
}

#[derive(Debug, Deserialize)]
struct OAuthTokens {
    #[serde(rename = "https://identity.mozilla.com/apps/oldsync")]
    oldsync: Option<OAuthToken>,
}

#[derive(Debug, Deserialize)]
struct OAuthToken {
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignedInUser {
    version: u32,
    account_data: FirefoxAccountData,
}

/// Firefox Sync API客户端
pub struct FirefoxSyncAPIClient {
    token: String,
    uid: String,
    email: String,
    sync_url: String,
}

impl FirefoxSyncAPIClient {
    /// 从Waterfox profile加载认证信息
    pub fn from_profile(profile_path: &Path) -> Result<Self> {
        let signed_in_user_path = profile_path.join("signedInUser.json");

        if !signed_in_user_path.exists() {
            bail!("Not signed in to Firefox Account");
        }

        let content =
            fs::read_to_string(&signed_in_user_path).context("Failed to read signedInUser.json")?;

        let user: SignedInUser =
            serde_json::from_str(&content).context("Failed to parse signedInUser.json")?;

        let token = user
            .account_data
            .oauth_tokens
            .oldsync
            .ok_or_else(|| anyhow::anyhow!("No oldsync token found"))?
            .token;

        info!("✅ Loaded Firefox Account: {}", user.account_data.email);

        Ok(Self {
            token,
            uid: user.account_data.uid,
            email: user.account_data.email,
            sync_url: "https://token.services.mozilla.com".to_string(),
        })
    }

    /// 获取Sync存储端点
    async fn get_sync_endpoint(&self) -> Result<SyncEndpoint> {
        let client = reqwest::Client::new();

        let url = format!("{}/1.0/sync/1.5", self.sync_url);

        info!("🔍 Getting Sync endpoint...");
        debug!("   URL: {}", url);

        // Firefox Sync需要X-KeyID头
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-KeyID", &self.uid)
            .send()
            .await
            .context("Failed to get sync endpoint")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Failed to get sync endpoint: {} - {}", status, body);
        }

        let endpoint: SyncEndpoint = response
            .json()
            .await
            .context("Failed to parse sync endpoint response")?;

        info!("✅ Sync endpoint: {}", endpoint.api_endpoint);

        Ok(endpoint)
    }

    /// 上传书签到云端
    pub async fn upload_bookmarks(&self, bookmarks: &[crate::browsers::Bookmark]) -> Result<()> {
        info!("📤 Uploading bookmarks to Firefox Sync cloud...");

        // 1. 获取Sync端点
        let endpoint = self.get_sync_endpoint().await?;

        // 2. 转换书签格式为Firefox Sync格式
        let sync_bookmarks = self.convert_to_sync_format(bookmarks)?;

        // 3. 上传到云端
        self.upload_to_cloud(&endpoint, sync_bookmarks).await?;

        info!("✅ Bookmarks uploaded to cloud successfully");

        Ok(())
    }

    /// 转换书签格式
    fn convert_to_sync_format(
        &self,
        bookmarks: &[crate::browsers::Bookmark],
    ) -> Result<Vec<SyncBookmark>> {
        info!("🔄 Converting bookmarks to Sync format...");

        let mut sync_bookmarks = Vec::new();
        Self::convert_recursive(bookmarks, "menu", &mut sync_bookmarks)?;

        info!("   Converted {} bookmarks", sync_bookmarks.len());

        Ok(sync_bookmarks)
    }

    /// 递归转换书签
    fn convert_recursive(
        bookmarks: &[crate::browsers::Bookmark],
        parent_id: &str,
        output: &mut Vec<SyncBookmark>,
    ) -> Result<()> {
        for bookmark in bookmarks {
            let id = format!("{}_{}", parent_id, bookmark.id);

            if bookmark.folder {
                // 文件夹
                output.push(SyncBookmark {
                    id: id.clone(),
                    type_field: "folder".to_string(),
                    parent_id: parent_id.to_string(),
                    title: bookmark.title.clone(),
                    children: bookmark
                        .children
                        .iter()
                        .map(|c| format!("{}_{}", id, c.id))
                        .collect(),
                    ..Default::default()
                });

                // 递归处理子项
                Self::convert_recursive(&bookmark.children, &id, output)?;
            } else if let Some(ref url) = bookmark.url {
                // 书签
                output.push(SyncBookmark {
                    id: id.clone(),
                    type_field: "bookmark".to_string(),
                    parent_id: parent_id.to_string(),
                    title: bookmark.title.clone(),
                    bmk_uri: Some(url.clone()),
                    ..Default::default()
                });
            }
        }

        Ok(())
    }

    /// 上传到云端
    async fn upload_to_cloud(
        &self,
        endpoint: &SyncEndpoint,
        bookmarks: Vec<SyncBookmark>,
    ) -> Result<()> {
        let client = reqwest::Client::new();

        // Firefox Sync使用批量上传
        let batch_size = 100;
        let total = bookmarks.len();

        info!(
            "📤 Uploading {} bookmarks in batches of {}...",
            total, batch_size
        );

        for (i, chunk) in bookmarks.chunks(batch_size).enumerate() {
            let url = format!("{}/storage/bookmarks", endpoint.api_endpoint);

            let payload: Vec<_> = chunk
                .iter()
                .map(|b| {
                    serde_json::json!({
                        "id": b.id,
                        "payload": serde_json::to_string(b).unwrap(),
                    })
                })
                .collect();

            debug!(
                "   Uploading batch {}/{}",
                i + 1,
                total.div_ceil(batch_size)
            );

            let response = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await
                .context("Failed to upload bookmarks")?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                bail!("Failed to upload bookmarks: {} - {}", status, body);
            }

            info!(
                "   ✅ Batch {}/{} uploaded",
                i + 1,
                total.div_ceil(batch_size)
            );
        }

        Ok(())
    }
}

/// Sync端点信息
#[derive(Debug, Deserialize)]
struct SyncEndpoint {
    api_endpoint: String,
    uid: String,
    duration: u64,
}

/// Firefox Sync书签格式
#[derive(Debug, Serialize, Default)]
struct SyncBookmark {
    id: String,
    #[serde(rename = "type")]
    type_field: String,
    #[serde(rename = "parentid")]
    parent_id: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "bmkUri")]
    bmk_uri: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<String>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_sync_api_client() {
        // 测试需要真实的profile
    }
}
