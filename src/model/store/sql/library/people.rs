
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
            
            conn.execute("DELETE FROM media_people_mapping WHERE people_ref = ?", &[&tag_id])?;
            conn.execute("DELETE FROM people_faces WHERE people_ref = ?", &[&tag_id])?;
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

    fn row_to_unassigned_face(row: &rusqlite::Row) -> rusqlite::Result<UnassignedFace> {
        let embedding_blob: Vec<u8> = row.get(1)?;
        let embedding = if embedding_blob.len() % 4 == 0 {
            bytemuck::cast_slice::<u8, f32>(&embedding_blob).to_vec()
        } else {
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
    }

    pub async fn get_all_unassigned_faces(&self, limit: Option<usize>, created_before: Option<i64>) -> Result<Vec<UnassignedFace>> {
        // If both are None, return all faces (for internal calls)
        // Otherwise, use default limit of 50 for API calls
        let get_all = limit.is_none() && created_before.is_none();
        let limit_val = limit.unwrap_or(50);
        let created_before_val = created_before;
        let res = self.connection.call(move |conn| {
            let faces = match (created_before_val, get_all) {
                (Some(created_before_ts), false) => {
                    // Query with WHERE clause and LIMIT for pagination
                    
                    let mut stmt = conn.prepare(
                        "SELECT id, embedding, media_ref, bbox, confidence, pose, cluster_id, created 
                            FROM unassigned_faces 
                            WHERE created < ? 
                            ORDER BY created DESC 
                            LIMIT ?"
                    )?;
                    let created_before_param = created_before_ts;
                    let limit_param = limit_val as i64;
                    let rows = stmt.query_map(params![created_before_param, limit_param], Self::row_to_unassigned_face)?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                    
                },
                (None, false) => {
                    // Query without WHERE clause but with LIMIT (first page of API call)
                    let mut stmt = conn.prepare(
                        "SELECT id, embedding, media_ref, bbox, confidence, pose, cluster_id, created 
                            FROM unassigned_faces 
                            ORDER BY created DESC 
                            LIMIT ?"
                    )?;
                    let limit_param = limit_val as i64;
                    let rows = stmt.query_map(params![limit_param], Self::row_to_unassigned_face)?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                },
                (_, true) => {
                    // Query without WHERE clause and without LIMIT (get all faces for internal calls)
                    let mut stmt = conn.prepare(
                        "SELECT id, embedding, media_ref, bbox, confidence, pose, cluster_id, created 
                         FROM unassigned_faces 
                         ORDER BY created DESC"
                    )?;
                    let rows = stmt.query_map([], Self::row_to_unassigned_face)?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                }
            };
            
            Ok(faces)
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
        pose: Option<(f32, f32, f32)>,
        similarity: Option<f32>
    ) -> Result<()> {
        let pid = person_id.to_string();
        let media_id_clone = media_id.clone();
        let confidence_int = (confidence * 100.0) as i32;
        self.connection.call(move |conn| {
            let tx = conn.transaction()?;
            
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

            // Insert into people_faces
            tx.execute(
                "INSERT INTO people_faces (id, people_ref, embedding, media_ref, bbox, confidence, pose, similarity, created) 
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![face_id, pid.clone(), embedding_blob, media_id_clone.clone(), bbox_json, confidence, pose_json, similarity, chrono::Utc::now().timestamp_millis()]
            )?;
            
            // Also insert or update media_people_mapping if media_id is provided
            if let Some(ref m_id) = media_id_clone {
                tx.execute(
                    "INSERT OR REPLACE INTO media_people_mapping (media_ref, people_ref, confidence, people_face_ref, similarity) VALUES (?, ?, ?, ?, ?)",
                    params![m_id, pid, confidence_int, face_id.clone(), similarity]
                )?;
            }
            
            tx.commit()?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn get_person_embeddings(&self, person_id: &str) -> Result<Vec<FaceEmbedding>> {
        let pid = person_id.to_string();
        let res = self.connection.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, embedding, media_ref, bbox, confidence, pose, people_ref 
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
                    person_id: Some(row.get(6)?),
                })
            })?;
            
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await?;
        Ok(res)
    }

    pub async fn get_highest_confidence_face(&self, person_id: &str) -> Result<Option<FaceEmbedding>> {
        let pid = person_id.to_string();
        let res = self.connection.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, embedding, media_ref, bbox, confidence, pose, people_ref 
                 FROM people_faces WHERE people_ref = ? ORDER BY confidence DESC LIMIT 1"
            )?;
            
            let row = stmt.query_row(params![pid], |row| {
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
                    person_id: Some(row.get(6)?),
                })
            }).optional()?;
            
            Ok(row)
        }).await?;
        Ok(res)
    }

    pub async fn get_media_embeddings(&self, media_id: &str) -> Result<Vec<FaceEmbedding>> {
        let media_id_str = media_id.to_string();
        let res = self.connection.call(move |conn| {
            let mut all_faces = Vec::new();
            
            // Query assigned faces from people_faces
            {
                let mut stmt = conn.prepare(
                    "SELECT id, embedding, media_ref, bbox, confidence, pose, people_ref 
                     FROM people_faces WHERE media_ref = ?"
                )?;
                
                let rows = stmt.query_map(params![media_id_str.clone()], |row| {
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
                        person_id: Some(row.get(6)?),
                    })
                })?;
                
                for face in rows {
                    all_faces.push(face?);
                }
            }
            
            // Query unassigned faces from unassigned_faces
            {
                let mut stmt = conn.prepare(
                    "SELECT id, embedding, media_ref, bbox, confidence, pose 
                     FROM unassigned_faces WHERE media_ref = ?"
                )?;
                
                let rows = stmt.query_map(params![media_id_str], |row| {
                    let embedding_blob: Vec<u8> = row.get(1)?;
                    let embedding = if embedding_blob.len() % 4 == 0 {
                        bytemuck::cast_slice::<u8, f32>(&embedding_blob).to_vec()
                    } else {
                        Vec::new()
                    };
                    
                    let bbox_str: String = row.get(3)?;
                    let bbox = serde_json::from_str(&bbox_str).ok();

                    let pose_str: Option<String> = row.get(5)?;
                    let pose = pose_str.and_then(|s| serde_json::from_str(&s).ok());

                    Ok(FaceEmbedding {
                        id: row.get(0)?,
                        embedding,
                        media_ref: Some(row.get(2)?),
                        bbox,
                        confidence: Some(row.get(4)?),
                        pose,
                        person_id: None,
                    })
                })?;
                
                for face in rows {
                    all_faces.push(face?);
                }
            }
            
            Ok(all_faces)
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
            let rows_affected = conn.execute("DELETE FROM people_faces WHERE id = ?", params![fid])?;
            if rows_affected == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows.into());
            }
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn delete_unassigned_face(&self, face_id: &str) -> Result<()> {
        let fid = face_id.to_string();
        self.connection.call(move |conn| {
            let rows_affected = conn.execute("DELETE FROM unassigned_faces WHERE id = ?", params![fid])?;
            if rows_affected == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows.into());
            }
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn get_face_by_id(&self, face_id: &str) -> Result<Option<(String, Option<FaceBBox>)>> {
        let fid = face_id.to_string();
        let res = self.connection.call(move |conn| {
            // Try people_faces first
            let mut stmt = conn.prepare("SELECT media_ref, bbox FROM people_faces WHERE id = ?")?;
            let result: Option<(String, Option<FaceBBox>)> = stmt.query_row(params![fid.clone()], |row| {
                let media_ref: Option<String> = row.get(0)?;
                let bbox_str: Option<String> = row.get(1)?;
                let bbox = bbox_str.and_then(|s| serde_json::from_str(&s).ok());
                Ok((media_ref.unwrap_or_default(), bbox))
            }).optional()?;
            
            if result.is_some() {
                return Ok(result);
            }
            
            // If not found, try unassigned_faces
            let mut stmt = conn.prepare("SELECT media_ref, bbox FROM unassigned_faces WHERE id = ?")?;
            let result: Option<(String, Option<FaceBBox>)> = stmt.query_row(params![fid], |row| {
                let media_ref: String = row.get(0)?;
                let bbox_str: String = row.get(1)?;
                let bbox = serde_json::from_str(&bbox_str).ok();
                Ok((media_ref, bbox))
            }).optional()?;
            
            Ok(result)
        }).await?;
        Ok(res)
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

    pub async fn mark_faces_as_processed(&self, face_ids: Vec<String>) -> Result<()> {
        self.connection.call(move |conn| {
            let tx = conn.transaction()?;
            {
                let mut stmt = tx.prepare("UPDATE unassigned_faces SET processed = 1 WHERE id = ?")?;
                for fid in face_ids {
                    stmt.execute(params![fid])?;
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
                let mut stmt = tx.prepare("INSERT INTO people_faces (id, people_ref, embedding, media_ref, bbox, confidence, pose, similarity, created) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")?;
                for (id, emb, mref, bbox, conf, pose, created) in &faces {
                    stmt.execute(params![id, pid.clone(), emb, mref, bbox, conf, pose, None::<f32>, created])?;
                }
            }

            // 3. Insert or update media_people_mapping for unique media_refs
            {
                use std::collections::HashMap;
                let mut media_to_face: HashMap<String, String> = HashMap::new();
                // Track first face_id for each media_ref
                for (face_id, _, mref, _, _, _, _) in &faces {
                    media_to_face.entry(mref.clone()).or_insert_with(|| face_id.clone());
                }
                
                let mut stmt = tx.prepare("INSERT OR REPLACE INTO media_people_mapping (media_ref, people_ref, confidence, people_face_ref, similarity) VALUES (?, ?, ?, ?, ?)")?;
                for (media_ref, face_id) in media_to_face {
                    // Use default confidence of 100 for cluster promotions
                    // Similarity is NULL for cluster promotions (no automatic matching)
                    stmt.execute(params![media_ref, pid.clone(), 100, face_id, None::<f32>])?;
                }
            }

            // 4. Delete from unassigned_faces
            tx.execute("DELETE FROM unassigned_faces WHERE cluster_id = ?", params![cluster_id])?;

            tx.commit()?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn assign_unassigned_face_to_person(&self, face_id: String, person_id: String) -> Result<()> {
        let fid = face_id.clone();
        let pid = person_id.clone();
        self.connection.call(move |conn| {
            let tx = conn.transaction()?;
            
            // 1. Get the unassigned face
            let mut face_data: Option<(Vec<u8>, String, String, f32, Option<String>, i64)> = None;
            {
                let mut stmt = tx.prepare("SELECT embedding, media_ref, bbox, confidence, pose, created FROM unassigned_faces WHERE id = ?")?;
                let result = stmt.query_row(params![fid.clone()], |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f32>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                }).optional()?;
                
                if result.is_none() {
                    return Err(rusqlite::Error::QueryReturnedNoRows.into());
                }
                face_data = result;
            }
            
            let (embedding_blob, media_ref, bbox_str, confidence, pose_json, created) = face_data.unwrap();
            
            // 2. Insert into people_faces
            {
                let mut stmt = tx.prepare("INSERT INTO people_faces (id, people_ref, embedding, media_ref, bbox, confidence, pose, similarity, created) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")?;
                stmt.execute(params![fid.clone(), pid.clone(), embedding_blob, media_ref.clone(), bbox_str, confidence, pose_json, 1.0, created])?;
            }
            
            // 3. Insert or update media_people_mapping
            {
                let confidence_int = (confidence * 100.0) as i32;
                tx.execute(
                    "INSERT OR REPLACE INTO media_people_mapping (media_ref, people_ref, confidence, people_face_ref, similarity) VALUES (?, ?, ?, ?, ?)",
                    params![media_ref, pid, confidence_int, fid.clone(), 1.0]
                )?;
            }
            
            // 4. Delete from unassigned_faces
            tx.execute("DELETE FROM unassigned_faces WHERE id = ?", params![fid])?;
            
            tx.commit()?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn assign_unassigned_faces_to_person_batch(&self, face_ids: Vec<String>, person_id: String) -> Result<usize> {
        let pid = person_id.clone();
        let face_ids_clone = face_ids.clone();
        let res = self.connection.call(move |conn| {
            let tx = conn.transaction()?;
            
            let mut assigned_count = 0;
            
            // Process each face
            for face_id in &face_ids_clone {
                // 1. Get the unassigned face
                let mut face_data: Option<(Vec<u8>, String, String, f32, Option<String>, i64)> = None;
                {
                    let mut stmt = tx.prepare("SELECT embedding, media_ref, bbox, confidence, pose, created FROM unassigned_faces WHERE id = ?")?;
                    let result = stmt.query_row(params![face_id], |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, f32>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    }).optional()?;
                    
                    if result.is_none() {
                        continue; // Skip if face not found
                    }
                    face_data = result;
                }
                
                let (embedding_blob, media_ref, bbox_str, confidence, pose_json, created) = face_data.unwrap();
                
                // 2. Insert into people_faces
                {
                    let mut stmt = tx.prepare("INSERT INTO people_faces (id, people_ref, embedding, media_ref, bbox, confidence, pose, similarity, created) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")?;
                    stmt.execute(params![face_id, pid.clone(), embedding_blob, media_ref.clone(), bbox_str, confidence, pose_json, 1.0, created])?;
                }
                
                // 3. Insert or update media_people_mapping
                {
                    let confidence_int = (confidence * 100.0) as i32;
                    tx.execute(
                        "INSERT OR REPLACE INTO media_people_mapping (media_ref, people_ref, confidence, people_face_ref, similarity) VALUES (?, ?, ?, ?, ?)",
                        params![media_ref, pid.clone(), confidence_int, face_id, 1.0]
                    )?;
                }
                
                // 4. Delete from unassigned_faces
                tx.execute("DELETE FROM unassigned_faces WHERE id = ?", params![face_id])?;
                
                assigned_count += 1;
            }
            
            tx.commit()?;
            Ok(assigned_count)
        }).await?;
        Ok(res)
    }

    pub async fn unassign_faces_from_person_batch(&self, face_ids: Vec<String>) -> Result<usize> {
        use crate::domain::people::FaceBBox;
        let face_ids_clone = face_ids.clone();
        let res = self.connection.call(move |conn| {
            let tx = conn.transaction()?;
            
            let mut unassigned_count = 0;
            
            // Process each face
            for face_id in &face_ids_clone {
                // 1. Get the assigned face from people_faces
                let mut face_data: Option<(Vec<u8>, Option<String>, Option<String>, f32, Option<String>, i64)> = None;
                {
                    let mut stmt = tx.prepare("SELECT embedding, media_ref, bbox, confidence, pose, created FROM people_faces WHERE id = ?")?;
                    let result = stmt.query_row(params![face_id], |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, f32>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    }).optional()?;
                    
                    if result.is_none() {
                        continue; // Skip if face not found
                    }
                    face_data = result;
                }
                
                let (embedding_blob, media_ref_opt, bbox_str_opt, confidence, pose_json, created) = face_data.unwrap();
                
                // Skip if media_ref is None (required in unassigned_faces)
                let media_ref = match media_ref_opt {
                    Some(ref_val) if !ref_val.is_empty() => ref_val,
                    _ => continue, // Skip faces without media_ref
                };
                
                // Handle bbox - use default if None, deserialize if Some
                let bbox_str = match bbox_str_opt {
                    Some(ref bbox_json) if !bbox_json.is_empty() => {
                        // Validate it's valid JSON, use default if not
                        match serde_json::from_str::<FaceBBox>(bbox_json) {
                            Ok(_) => bbox_json.clone(),
                            Err(_) => {
                                // Invalid JSON, use default
                                serde_json::to_string(&FaceBBox::default()).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                            }
                        }
                    },
                    _ => {
                        // None or empty, use default
                        serde_json::to_string(&FaceBBox::default()).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                    }
                };
                
                // 2. Insert into unassigned_faces
                {
                    let mut stmt = tx.prepare("INSERT INTO unassigned_faces (id, embedding, media_ref, bbox, confidence, pose, created) VALUES (?, ?, ?, ?, ?, ?, ?)")?;
                    stmt.execute(params![face_id, embedding_blob, media_ref.clone(), bbox_str.clone(), confidence, pose_json, created])?;
                }
                
                // 3. Delete from media_people_mapping where people_face_ref matches
                tx.execute("DELETE FROM media_people_mapping WHERE people_face_ref = ?", params![face_id])?;
                
                // 4. Delete from people_faces
                tx.execute("DELETE FROM people_faces WHERE id = ?", params![face_id])?;
                
                unassigned_count += 1;
            }
            
            tx.commit()?;
            Ok(unassigned_count)
        }).await?;
        Ok(res)
    }

    pub async fn mark_media_face_processed(&self, media_id: String) -> Result<()> {
        self.connection.call(move |conn| {
            conn.execute("UPDATE medias SET face_processed = 1 WHERE id = ?", params![media_id])?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn transfer_faces_between_people(&self, source_person_id: &str, target_person_id: &str) -> Result<usize> {
        let source_id = source_person_id.to_string();
        let target_id = target_person_id.to_string();
        let res = self.connection.call(move |conn| {
            let tx = conn.transaction()?;
            
            // 1. Update people_faces table
            let faces_affected = tx.execute(
                "UPDATE people_faces SET people_ref = ? WHERE people_ref = ?",
                params![target_id.clone(), source_id.clone()]
            )?;
            
            // 2. Handle media_people_mapping:
            //    - Delete mappings where both source and target exist for the same media (avoid duplicates)
            //    - Update remaining source mappings to target
            tx.execute(
                "DELETE FROM media_people_mapping 
                 WHERE people_ref = ? 
                 AND media_ref IN (
                     SELECT media_ref FROM media_people_mapping WHERE people_ref = ?
                 )",
                params![source_id.clone(), target_id.clone()]
            )?;
            
            // Update remaining source mappings to target
            let mappings_affected = tx.execute(
                "UPDATE media_people_mapping SET people_ref = ? WHERE people_ref = ?",
                params![target_id, source_id]
            )?;
            
            tx.commit()?;
            Ok(faces_affected)
        }).await?;
        Ok(res)
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
            let mut stmt = conn.prepare("SELECT id FROM medias WHERE face_processed = 0 ORDER BY added DESC LIMIT ?")?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(row.get::<_, String>(0)?)
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await?;
        Ok(res)
    }
}
