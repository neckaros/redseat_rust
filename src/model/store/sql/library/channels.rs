use rs_plugin_common_interfaces::ImageType;
use rusqlite::{params, OptionalExtension, Row};

use crate::domain::channel::{Channel, ChannelForUpdate, ChannelVariant};

use super::{Result, SqliteLibraryStore};

const CHANNEL_SELECT: &str = "SELECT c.id, c.name, c.tvg_id, c.logo, c.channel_number, c.posterv, c.modified, c.added, (SELECT GROUP_CONCAT(tag_ref) FROM channel_tag_mapping WHERE channel_ref = c.id) AS tags FROM channels c";

impl SqliteLibraryStore {
    fn row_to_channel(row: &Row) -> rusqlite::Result<Channel> {
        let tags_raw: Option<String> = row.get(8)?;
        let tags = tags_raw.map(|s| s.split(',').map(|t| t.to_string()).collect::<Vec<_>>());
        Ok(Channel {
            id: row.get(0)?,
            name: row.get(1)?,
            tvg_id: row.get(2)?,
            logo: row.get(3)?,
            tags,
            channel_number: row.get(4)?,
            posterv: row.get(5)?,
            modified: row.get(6)?,
            added: row.get(7)?,
            variants: None,
        })
    }

    fn row_to_variant(row: &Row) -> rusqlite::Result<ChannelVariant> {
        Ok(ChannelVariant {
            id: row.get(0)?,
            channel_ref: row.get(1)?,
            quality: row.get(2)?,
            stream_url: row.get(3)?,
            name: row.get(4)?,
            tvg_name: row.get(5)?,
            modified: row.get(6)?,
            added: row.get(7)?,
        })
    }

    pub async fn get_channels(
        &self,
        tag: Option<String>,
        name_filter: Option<String>,
    ) -> Result<Vec<Channel>> {
        let rows = self
            .connection
            .call(move |conn| {
                let mut sql = CHANNEL_SELECT.to_string();
                let mut conditions: Vec<String> = Vec::new();
                let mut values: Vec<Box<dyn rusqlite::types::ToSql + Send>> = Vec::new();

                if let Some(ref t) = tag {
                    conditions.push(format!("c.id IN (SELECT channel_ref FROM channel_tag_mapping WHERE tag_ref = ?{})", conditions.len() + 1));
                    values.push(Box::new(t.clone()));
                }
                if let Some(ref name) = name_filter {
                    conditions.push(format!("c.name LIKE ?{}", conditions.len() + 1));
                    values.push(Box::new(format!("%{}%", name)));
                }

                if !conditions.is_empty() {
                    sql.push_str(" WHERE ");
                    sql.push_str(&conditions.join(" AND "));
                }
                sql.push_str(" ORDER BY c.channel_number ASC, c.name ASC");

                let mut statement = conn.prepare(&sql)?;
                let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref() as &dyn rusqlite::types::ToSql).collect();
                let rows = statement.query_map(params.as_slice(), Self::row_to_channel)?;
                let channels = rows.collect::<std::result::Result<Vec<Channel>, rusqlite::Error>>()?;
                Ok(channels)
            })
            .await?;
        Ok(rows)
    }

    pub async fn get_channel(&self, channel_id: &str) -> Result<Option<Channel>> {
        let channel_id = channel_id.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let sql = format!("{} WHERE c.id = ?", CHANNEL_SELECT);
                let mut statement = conn.prepare(&sql)?;
                let row = statement
                    .query_row([channel_id], Self::row_to_channel)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_channel_by_tvg_id(&self, tvg_id: &str) -> Result<Option<Channel>> {
        let tvg_id = tvg_id.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let sql = format!("{} WHERE c.tvg_id = ? LIMIT 1", CHANNEL_SELECT);
                let mut statement = conn.prepare(&sql)?;
                let row = statement
                    .query_row([tvg_id], Self::row_to_channel)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_channel_by_name(&self, name: &str) -> Result<Option<Channel>> {
        let name = name.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let sql = format!("{} WHERE c.name = ? LIMIT 1", CHANNEL_SELECT);
                let mut statement = conn.prepare(&sql)?;
                let row = statement
                    .query_row([name], Self::row_to_channel)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn add_channel(&self, channel: Channel) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO channels (id, name, tvg_id, logo, channel_number) VALUES (?, ?, ?, ?, ?)",
                    params![
                        channel.id,
                        channel.name,
                        channel.tvg_id,
                        channel.logo,
                        channel.channel_number,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn update_channel(&self, channel_id: &str, update: ChannelForUpdate) -> Result<()> {
        let channel_id = channel_id.to_string();
        self.connection
            .call(move |conn| {
                let mut sets: Vec<String> = Vec::new();
                let mut values: Vec<Box<dyn rusqlite::types::ToSql + Send>> = Vec::new();
                let mut idx = 1;

                if let Some(ref name) = update.name {
                    sets.push(format!("name = ?{}", idx));
                    values.push(Box::new(name.clone()));
                    idx += 1;
                }
                if let Some(ref tvg_id) = update.tvg_id {
                    sets.push(format!("tvg_id = ?{}", idx));
                    values.push(Box::new(tvg_id.clone()));
                    idx += 1;
                }
                if let Some(ref logo) = update.logo {
                    sets.push(format!("logo = ?{}", idx));
                    values.push(Box::new(logo.clone()));
                    idx += 1;
                }
                if let Some(channel_number) = update.channel_number {
                    sets.push(format!("channel_number = ?{}", idx));
                    values.push(Box::new(channel_number));
                    idx += 1;
                }

                if !sets.is_empty() {
                    let sql = format!(
                        "UPDATE channels SET {} WHERE id = ?{}",
                        sets.join(", "),
                        idx
                    );
                    values.push(Box::new(channel_id));
                    let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref() as &dyn rusqlite::types::ToSql).collect();
                    conn.execute(&sql, params.as_slice())?;
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn update_channel_image(&self, channel_id: String, _kind: ImageType) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute(
                    "UPDATE channels SET posterv = ifnull(posterv, 0) + 1 WHERE id = ?",
                    params![channel_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn remove_channel(&self, channel_id: String) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute("DELETE FROM channel_tag_mapping WHERE channel_ref = ?", [&channel_id])?;
                conn.execute("DELETE FROM channel_variants WHERE channel_ref = ?", [&channel_id])?;
                conn.execute("DELETE FROM channels WHERE id = ?", [&channel_id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn get_all_channel_ids(&self) -> Result<Vec<String>> {
        let ids = self
            .connection
            .call(move |conn| {
                let mut statement = conn.prepare("SELECT id FROM channels")?;
                let rows = statement.query_map([], |row| row.get(0))?;
                let ids = rows.collect::<std::result::Result<Vec<String>, rusqlite::Error>>()?;
                Ok(ids)
            })
            .await?;
        Ok(ids)
    }

    // --- Channel Tag Mapping ---

    pub async fn add_channel_tag(&self, channel_id: &str, tag_id: &str, confidence: Option<i32>) -> Result<()> {
        let channel_id = channel_id.to_string();
        let tag_id = tag_id.to_string();
        self.connection
            .call(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO channel_tag_mapping (channel_ref, tag_ref, confidence) VALUES (?, ?, ?)",
                    params![channel_id, tag_id, confidence],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn remove_channel_tag(&self, channel_id: &str, tag_id: &str) -> Result<()> {
        let channel_id = channel_id.to_string();
        let tag_id = tag_id.to_string();
        self.connection
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM channel_tag_mapping WHERE channel_ref = ? AND tag_ref = ?",
                    params![channel_id, tag_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn get_channel_auto_tag_ids(&self, channel_id: &str) -> Result<Vec<String>> {
        let channel_id = channel_id.to_string();
        let ids = self
            .connection
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT tag_ref FROM channel_tag_mapping WHERE channel_ref = ? AND confidence = 0",
                )?;
                let rows = statement.query_map([channel_id], |row| row.get(0))?;
                let ids = rows.collect::<std::result::Result<Vec<String>, rusqlite::Error>>()?;
                Ok(ids)
            })
            .await?;
        Ok(ids)
    }

    pub async fn remove_channel_auto_tag(&self, channel_id: &str, tag_id: &str) -> Result<()> {
        let channel_id = channel_id.to_string();
        let tag_id = tag_id.to_string();
        self.connection
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM channel_tag_mapping WHERE channel_ref = ? AND tag_ref = ? AND confidence = 0",
                    params![channel_id, tag_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    // --- Channel Variants ---

    pub async fn get_channel_variants(&self, channel_ref: &str) -> Result<Vec<ChannelVariant>> {
        let channel_ref = channel_ref.to_string();
        let rows = self
            .connection
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT id, channel_ref, quality, stream_url, name, tvg_name, modified, added FROM channel_variants WHERE channel_ref = ? ORDER BY quality ASC",
                )?;
                let rows = statement.query_map([channel_ref], Self::row_to_variant)?;
                let variants = rows.collect::<std::result::Result<Vec<ChannelVariant>, rusqlite::Error>>()?;
                Ok(variants)
            })
            .await?;
        Ok(rows)
    }

    pub async fn add_channel_variant(&self, variant: ChannelVariant) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO channel_variants (id, channel_ref, quality, stream_url, name, tvg_name) VALUES (?, ?, ?, ?, ?, ?)",
                    params![
                        variant.id,
                        variant.channel_ref,
                        variant.quality,
                        variant.stream_url,
                        variant.name,
                        variant.tvg_name,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn get_channel_variant_by_tvg_name(
        &self,
        channel_ref: &str,
        tvg_name: &str,
    ) -> Result<Option<ChannelVariant>> {
        let channel_ref = channel_ref.to_string();
        let tvg_name = tvg_name.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT id, channel_ref, quality, stream_url, name, tvg_name, modified, added FROM channel_variants WHERE channel_ref = ? AND tvg_name = ? LIMIT 1",
                )?;
                let row = statement
                    .query_row(params![channel_ref, tvg_name], Self::row_to_variant)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn remove_channel_variant(&self, variant_id: &str) -> Result<()> {
        let variant_id = variant_id.to_string();
        self.connection
            .call(move |conn| {
                conn.execute("DELETE FROM channel_variants WHERE id = ?", [&variant_id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn remove_channel_variants_for_channel(&self, channel_ref: &str) -> Result<()> {
        let channel_ref = channel_ref.to_string();
        self.connection
            .call(move |conn| {
                conn.execute("DELETE FROM channel_variants WHERE channel_ref = ?", [&channel_ref])?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}
