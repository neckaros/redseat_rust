
use rs_plugin_common_interfaces::domain::rs_ids::RsIds;
use rusqlite::{params, types::FromSqlError, OptionalExtension, Row};


use crate::{domain::people::{FaceBBox, FaceEmbedding, Person, UnassignedFace}, model::{people::{PeopleQuery, PersonForInsert, PersonForUpdate}, store::{from_pipe_separated_optional, sql::{deserialize_from_row, OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType}, to_pipe_separated_optional}}, plugins::sources::error::SourcesError, tools::{array_tools::replace_add_remove_from_array, serialization::optional_serde_to_string}};
use super::{Result, SqliteLibraryStore};
use crate::model::Error;




impl SqliteLibraryStore {

    const PEOPLE_FIELDS: &str = "id, name, socials, type, alt, portrait, params, birthday, modified, added, posterv, generated, imdb, slug, tmdb, trakt, death, gender, country, bio";
  
    fn row_to_person(row: &Row) -> rusqlite::Result<Person> {
        Ok(Person {
            id: row.get(0)?,
            name: row.get(1)?,
            socials: deserialize_from_row(row, 2)?,
            kind: row.get(3)?,
            alt: from_pipe_separated_optional(row.get(4)?),
            portrait: row.get(5)?,
            params: row.get(6)?,
            birthday: row.get(7)?,
            modified: row.get(8)?,
            added: row.get(9)?,
            posterv: row.get(10)?,
            generated: row.get(11)?,

            imdb: row.get(12)?,
            slug: row.get(13)?,
            tmdb: row.get(14)?,
            trakt: row.get(15)?,

            death: row.get(16)?,
            gender: row.get(17)?,
            country: row.get(18)?,
            bio: row.get(19)?,
        })
    }


    pub async fn get_people(&self, query: PeopleQuery) -> Result<Vec<Person>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = RsQueryBuilder::new();
            if let Some(q) = query.after {
                where_query.add_where(SqlWhereType::After("modified".to_owned(), Box::new(q)));
            }
            if query.after.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC))
            } else {
                if query.name.is_some() {
                    where_query.add_oder(OrderBuilder::new("score".to_string(), SqlOrder::DESC));
                }
                where_query.add_oder(OrderBuilder::new("name".to_string(), SqlOrder::ASC))
            }
            

            let mut score = "".to_string();
            if let Some(q) = query.name {

                score = format!(",
(case 
when name = '{}' then 100 
when socials like '%\"id\":\"{}\"%'  then 20
when (alt like '%|{}|%' or  alt like '{}|%'  or  alt like '%|{}'  or alt = '{}' COLLATE NOCASE ) then 10
else 0 end) as score", q, q, q, q, q, q);
                let name_queries = vec![SqlWhereType::EqualWithAlt("name".to_owned(), "alt".to_owned(), "|".to_owned(), Box::new(q.clone())),
                SqlWhereType::Like("socials".to_owned(), Box::new(format!("%\"id\":\"{}\"%", q)))];
                where_query.add_where(SqlWhereType::Or(name_queries));
            }

            let mut query = conn.prepare(&format!("SELECT {}{}  FROM people {}{}", Self::PEOPLE_FIELDS, score, where_query.format(), where_query.format_order()))?;

            //println!("sql: {:?}", query.expanded_sql());

            let rows = query.query_map(
            where_query.values(), Self::row_to_person,
            )?;
            let backups:Vec<Person> = rows.collect::<std::result::Result<Vec<Person>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }
    pub async fn get_person(&self, credential_id: &str) -> Result<Option<Person>> {
        let credential_id = credential_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare(&format!("SELECT {} FROM people WHERE id = ?", Self::PEOPLE_FIELDS))?;
            let row = query.query_row(
            [credential_id],Self::row_to_person).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn get_person_by_external_id(&self, ids: RsIds) -> Result<Option<Person>> {
        
        //println!("{}, {}, {}, {}, {}",i.imdb.unwrap_or("zz".to_string()), i.slug.unwrap_or("zz".to_string()), i.tmdb.unwrap_or(0), i.trakt.unwrap_or(0), i.tvdb.unwrap_or(0));
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare(&format!("SELECT  
            {}
            FROM people 
            WHERE 
            imdb = ? or slug = ? or tmdb = ? or trakt = ?", Self::PEOPLE_FIELDS))?;
            let row = query.query_row(
            params![ids.imdb.unwrap_or("zz".to_string()), ids.slug.unwrap_or("zz".to_string()), ids.tmdb.unwrap_or(0), ids.trakt.unwrap_or(0)],Self::row_to_person).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }


    pub async fn update_person(&self, person_id: &str, update: PersonForUpdate) -> Result<()> {
        let id = person_id.to_string();

        let existing = self.get_person(&person_id).await?.ok_or_else( || SourcesError::UnableToFindPerson("store".to_string() ,person_id.to_string(), "update_person".to_string()))?;


        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();

            where_query.add_update(&update.name, "name");
            where_query.add_update(&update.kind, "type");
            where_query.add_update(&update.portrait, "portrait");
            where_query.add_update(&update.params, "params");
            where_query.add_update(&update.birthday, "birthday");
            where_query.add_update(&update.generated, "generated");
            
            where_query.add_update(&update.imdb, "imdb");
            where_query.add_update(&update.slug, "slug");
            where_query.add_update(&update.tmdb, "tmdb");
            where_query.add_update(&update.trakt, "trakt");

            
            
            where_query.add_update(&update.bio, "bio");
            where_query.add_update(&update.gender, "gender");
            where_query.add_update(&update.death, "death");
            where_query.add_update(&update.country, "country");

            let alts = replace_add_remove_from_array(existing.alt, update.alt, update.add_alts, update.remove_alts);
            let v = to_pipe_separated_optional(alts);
            where_query.add_update(&v, "alt");
            println!("socialtsdd {:?}", v);

            let socials = replace_add_remove_from_array(existing.socials, update.socials, update.add_socials, update.remove_socials);
            let socials = optional_serde_to_string(socials).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            where_query.add_update(&socials, "socials");

            where_query.add_where(QueryWhereType::Equal("id", &id));
            

            let update_sql = format!("UPDATE people SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn update_person_portrait(&self, id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("update people set posterv = ifnull(posterv, 0) + 1 WHERE id = ?", params![id])?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn add_person(&self, person: PersonForInsert) -> Result<()> {
        self.connection.call( move |conn| { 
            
            let id = person.id;
            let person = person.person;
            let socials = if let Some(soc) = person.socials {
                Some(serde_json::to_string(&soc).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?)
            } else {
                None
            };

            conn.execute("INSERT INTO people (id, name, socials, type, alt, portrait, params, birthday, generated, imdb, slug, tmdb, trakt, death, gender, country, bio)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                id,
                person.name,
                socials,
                person.kind,
                to_pipe_separated_optional(person.alt),
                person.portrait,
                person.params,
                person.birthday,
                person.generated,

                person.imdb,
                person.slug,
                person.tmdb,
                person.trakt,
                
                person.death,
                person.gender,
                person.country,
                person.bio
                
            ])?;

            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_person(&self, tag_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("DELETE FROM people WHERE id = ?", &[&tag_id])?;
            Ok(())
        }).await?;
        Ok(())
    }

    // FACE RECOGNITION

    pub async fn add_unassigned_face(
        &self,
        face_id: String,
        embedding: Vec<f32>,
        media_id: String,
        bbox: FaceBBox,
        confidence: f32,
        pose: Option<(f32, f32, f32)>
    ) -> Result<()> {
        self.connection.call(move |conn| {
            let embedding_blob = bytemuck::cast_slice::<f32, u8>(&embedding).to_vec();
            let bbox_json = serde_json::to_string(&bbox).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let pose_json = if let Some(p) = pose {
                Some(serde_json::to_string(&p).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?)
            } else {
                None
            };
            
            conn.execute(
                "INSERT INTO unassigned_faces (id, embedding, media_ref, bbox, confidence, pose, created) 
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![face_id, embedding_blob, media_id, bbox_json, confidence, pose_json, chrono::Utc::now().timestamp_millis()]
            )?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn get_unassigned_faces(&self) -> Result<Vec<UnassignedFace>> {
        let res = self.connection.call(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, embedding, media_ref, bbox, confidence, pose, cluster_id, created 
                 FROM unassigned_faces WHERE processed = 0"
            )?;
            
            let rows = stmt.query_map([], |row| {
                let embedding_blob: Vec<u8> = row.get(1)?;
                // Safety: Validate blob size is multiple of f32 size
                let embedding = if embedding_blob.len() % 4 == 0 {
                    bytemuck::cast_slice::<u8, f32>(&embedding_blob).to_vec()
                } else {
                    // Corrupted data - return empty embedding
                    Vec::new()
                };
                
                let bbox_str: String = row.get(3)?;
                let bbox: FaceBBox = serde_json::from_str(&bbox_str).unwrap_or_default();

                let pose_str: Option<String> = row.get(5)?;
                let pose = pose_str.and_then(|s| serde_json::from_str(&s).ok());

                Ok(UnassignedFace {
                    id: row.get(0)?,
                    embedding,
                    media_ref: row.get(2)?,
                    bbox,
                    confidence: row.get(4)?,
                    pose,
                    cluster_id: row.get(6)?,
                    created: row.get(7)?,
                })
            })?;
            
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await?;
        Ok(res)
    }

    pub async fn add_face_embedding(
        &self,
        face_id: String,
        person_id: &str,
        embedding: Vec<f32>,
        media_id: Option<String>,
        bbox: Option<FaceBBox>,
        confidence: f32,
        pose: Option<(f32, f32, f32)>
    ) -> Result<()> {
        let pid = person_id.to_string();
        self.connection.call(move |conn| {
            let embedding_blob = bytemuck::cast_slice::<f32, u8>(&embedding).to_vec();
            let bbox_json = if let Some(b) = bbox {
                Some(serde_json::to_string(&b).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?)
            } else {
                None
            };
            let pose_json = if let Some(p) = pose {
                Some(serde_json::to_string(&p).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?)
            } else {
                None
            };

            conn.execute(
                "INSERT INTO people_faces (id, people_ref, embedding, media_ref, bbox, confidence, pose, created) 
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![face_id, pid, embedding_blob, media_id, bbox_json, confidence, pose_json, chrono::Utc::now().timestamp_millis()]
            )?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn get_person_embeddings(&self, person_id: &str) -> Result<Vec<FaceEmbedding>> {
        let pid = person_id.to_string();
        let res = self.connection.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, embedding, media_ref, bbox, confidence, pose 
                 FROM people_faces WHERE people_ref = ?"
            )?;
            
            let rows = stmt.query_map(params![pid], |row| {
                let embedding_blob: Vec<u8> = row.get(1)?;
                let embedding = if embedding_blob.len() % 4 == 0 {
                    bytemuck::cast_slice::<u8, f32>(&embedding_blob).to_vec()
                } else {
                    Vec::new()
                };
                
                let bbox_str: Option<String> = row.get(3)?;
                let bbox = bbox_str.and_then(|s| serde_json::from_str(&s).ok());

                let pose_str: Option<String> = row.get(5)?;
                let pose = pose_str.and_then(|s| serde_json::from_str(&s).ok());

                Ok(FaceEmbedding {
                    id: row.get(0)?,
                    embedding,
                    media_ref: row.get(2)?,
                    bbox,
                    confidence: row.get(4)?,
                    pose,
                })
            })?;
            
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await?;
        Ok(res)
    }

    pub async fn get_all_embeddings(&self) -> Result<Vec<(String, Vec<f32>)>> {
        let res = self.connection.call(|conn| {
            let mut stmt = conn.prepare("SELECT people_ref, embedding FROM people_faces")?;
            let rows = stmt.query_map([], |row| {
                let pid: String = row.get(0)?;
                let embedding_blob: Vec<u8> = row.get(1)?;
                let embedding = if embedding_blob.len() % 4 == 0 {
                    bytemuck::cast_slice::<u8, f32>(&embedding_blob).to_vec()
                } else {
                    Vec::new()
                };
                Ok((pid, embedding))
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await?;
        Ok(res)
    }

    pub async fn delete_face_embedding(&self, face_id: &str) -> Result<()> {
        let fid = face_id.to_string();
        self.connection.call(move |conn| {
            conn.execute("DELETE FROM people_faces WHERE id = ?", params![fid])?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn assign_cluster_to_faces(&self, face_ids: Vec<String>, cluster_id: String) -> Result<()> {
        self.connection.call(move |conn| {
            // SQLite doesn't support array params, so we iterate or use IN clause construction
            // Since this is likely small batch, transaction is fine
            let tx = conn.transaction()?;
            {
                let mut stmt = tx.prepare("UPDATE unassigned_faces SET cluster_id = ?, processed = 1 WHERE id = ?")?;
                for fid in face_ids {
                    stmt.execute(params![cluster_id, fid])?;
                }
            }
            tx.commit()?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn promote_cluster_to_person(&self, cluster_id: String, person_id: String) -> Result<()> {
        // Move faces from unassigned_faces to people_faces
        // Requires selecting, then inserting, then deleting
        let pid = person_id.to_string();
        self.connection.call(move |conn| {
            let tx = conn.transaction()?;
            
            // 1. Get faces
            let mut faces = Vec::new();
            {
                let mut stmt = tx.prepare("SELECT id, embedding, media_ref, bbox, confidence, pose, created FROM unassigned_faces WHERE cluster_id = ?")?;
                let rows = stmt.query_map(params![cluster_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, f32>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                })?;
                for r in rows {
                    faces.push(r?);
                }
            }

            // 2. Insert into people_faces
            {
                let mut stmt = tx.prepare("INSERT INTO people_faces (id, people_ref, embedding, media_ref, bbox, confidence, pose, created) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")?;
                for (id, emb, mref, bbox, conf, pose, created) in &faces {
                    stmt.execute(params![id, pid, emb, mref, bbox, conf, pose, created])?;
                }
            }

            // 3. Delete from unassigned_faces
            tx.execute("DELETE FROM unassigned_faces WHERE cluster_id = ?", params![cluster_id])?;

            tx.commit()?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn mark_media_face_processed(&self, media_id: String) -> Result<()> {
        self.connection.call(move |conn| {
            conn.execute("UPDATE medias SET face_processed = 1 WHERE id = ?", params![media_id])?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn update_media_people_mapping(&self, media_id: String, person_id: &str) -> Result<()> {
        let pid = person_id.to_string();
        self.connection.call(move |conn| {
            conn.execute("INSERT OR IGNORE INTO media_people_mapping (media_ref, people_ref, confidence) VALUES (?, ?, 100)", params![media_id, pid])?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn get_medias_for_face_processing(&self, limit: usize) -> Result<Vec<String>> {
        let limit = limit as i64;
        let res = self.connection.call(move |conn| {
            let mut stmt = conn.prepare("SELECT id FROM medias WHERE face_processed = 0 AND type = 'photo' LIMIT ?")?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(row.get::<_, String>(0)?)
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await?;
        Ok(res)
    }
}
