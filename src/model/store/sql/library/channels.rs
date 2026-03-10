use rs_plugin_common_interfaces::ImageType;
use rusqlite::{params, OptionalExtension, Row};

use crate::domain::channel::{Channel, ChannelForUpdate, ChannelVariant};

use super::{Result, SqliteLibraryStore};

impl SqliteLibraryStore {
    fn row_to_channel(row: &Row) -> rusqlite::Result<Channel> {
        Ok(Channel {
            id: row.get(0)?,
            name: row.get(1)?,
            tvg_id: row.get(2)?,
            logo: row.get(3)?,
            group_tag: row.get(4)?,
            channel_number: row.get(5)?,
            posterv: row.get(6)?,
            modified: row.get(7)?,
            added: row.get(8)?,
            variants: None,
        })
    }

    fn row_to_variant(row: &Row) -> rusqlite::Result<ChannelVariant> {
        Ok(ChannelVariant {
            id: row.get(0)?,
            channel_ref: row.get(1)?,
            quality: row.get(2)?,
            stream_url: row.get(3)?,
            modified: row.get(4)?,
            added: row.get(5)?,
        })
    }

    pub async fn get_channels(
        &self,
        group_tag: Option<String>,
        name_filter: Option<String>,
    ) -> Result<Vec<Channel>> {
        let rows = self
            .connection
            .call(move |conn| {
                let mut sql = "SELECT id, name, tvg_id, logo, group_tag, channel_number, posterv, modified, added FROM channels".to_string();
                let mut conditions: Vec<String> = Vec::new();
                let mut values: Vec<Box<dyn rusqlite::types::ToSql + Send>> = Vec::new();

                if let Some(ref gt) = group_tag {
                    conditions.push(format!("group_tag = ?{}", conditions.len() + 1));
                    values.push(Box::new(gt.clone()));
                }
                if let Some(ref name) = name_filter {
                    conditions.push(format!("name LIKE ?{}", conditions.len() + 1));
                    values.push(Box::new(format!("%{}%", name)));
                }

                if !conditions.is_empty() {
                    sql.push_str(" WHERE ");
                    sql.push_str(&conditions.join(" AND "));
                }
                sql.push_str(" ORDER BY channel_number ASC, name ASC");

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
                let mut statement = conn.prepare(
                    "SELECT id, name, tvg_id, logo, group_tag, channel_number, posterv, modified, added FROM channels WHERE id = ?",
                )?;
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
                let mut statement = conn.prepare(
                    "SELECT id, name, tvg_id, logo, group_tag, channel_number, posterv, modified, added FROM channels WHERE tvg_id = ? LIMIT 1",
                )?;
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
                let mut statement = conn.prepare(
                    "SELECT id, name, tvg_id, logo, group_tag, channel_number, posterv, modified, added FROM channels WHERE name = ? LIMIT 1",
                )?;
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
                    "INSERT OR REPLACE INTO channels (id, name, tvg_id, logo, group_tag, channel_number) VALUES (?, ?, ?, ?, ?, ?)",
                    params![
                        channel.id,
                        channel.name,
                        channel.tvg_id,
                        channel.logo,
                        channel.group_tag,
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
                if let Some(ref group_tag) = update.group_tag {
                    sets.push(format!("group_tag = ?{}", idx));
                    values.push(Box::new(group_tag.clone()));
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

    // --- Channel Variants ---

    pub async fn get_channel_variants(&self, channel_ref: &str) -> Result<Vec<ChannelVariant>> {
        let channel_ref = channel_ref.to_string();
        let rows = self
            .connection
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT id, channel_ref, quality, stream_url, modified, added FROM channel_variants WHERE channel_ref = ? ORDER BY quality ASC",
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
                    "INSERT OR REPLACE INTO channel_variants (id, channel_ref, quality, stream_url) VALUES (?, ?, ?, ?)",
                    params![
                        variant.id,
                        variant.channel_ref,
                        variant.quality,
                        variant.stream_url,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn get_channel_variant_by_quality(
        &self,
        channel_ref: &str,
        quality: &str,
    ) -> Result<Option<ChannelVariant>> {
        let channel_ref = channel_ref.to_string();
        let quality = quality.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT id, channel_ref, quality, stream_url, modified, added FROM channel_variants WHERE channel_ref = ? AND quality = ? LIMIT 1",
                )?;
                let row = statement
                    .query_row(params![channel_ref, quality], Self::row_to_variant)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
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
