use std::str::FromStr;

use rusqlite::{params, params_from_iter, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, OptionalExtension, ToSql};

use crate::{domain::credential::Credential, model::{credentials::CredentialForUpdate, store::SqliteStore}};
use super::Result;





impl SqliteStore {
  
    
    pub async fn get_credentials(&self) -> Result<Vec<Credential>> {
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, name, source, type, login, password, preferences, user_ref, refreshtoken, expires  FROM Credentials")?;
            let rows = query.query_map(
            [],
            |row| {
                Ok(Credential {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    source:  row.get(2)?,
                    kind:  row.get(3)?,
                    login:  row.get(4)?,
                    password:  row.get(5)?,
                    settings:  row.get(6)?,
                    user_ref:  row.get(7)?,
                    refresh_token:  row.get(8)?,
                    expires:  row.get(9)?,
                    
                })
            },
            )?;
            let credentials:Vec<Credential> = rows.collect::<std::result::Result<Vec<Credential>, rusqlite::Error>>()?; 
            Ok(credentials)
        }).await?;
        Ok(row)
    }

    pub async fn get_credential(&self, credential_id: &str) -> Result<Option<Credential>> {
        let credential_id = credential_id.to_string();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, name, source, type, login, password, preferences, user_ref, refreshtoken, expires  FROM Credentials WHERE id = ?")?;
            let row = query.query_row(
            [credential_id],
            |row| {
                Ok(Credential {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    source:  row.get(2)?,
                    kind:  row.get(3)?,
                    login:  row.get(4)?,
                    password:  row.get(5)?,
                    settings:  row.get(6)?,
                    user_ref:  row.get(7)?,
                    refresh_token:  row.get(8)?,
                    expires:  row.get(9)?,
                    
                })
            },
            ).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }



    pub async fn update_credentials(&self, credential_id: &str, update: CredentialForUpdate) -> Result<()> {
        let credential_id = credential_id.to_string();
        self.server_store.call( move |conn| { 
            let mut columns: Vec<String> = Vec::new();
            let mut values: Vec<Box<dyn ToSql>> = Vec::new();
            super::add_for_sql_update(update.name, "name", &mut columns, &mut values);
            super::add_for_sql_update(update.source, "source", &mut columns, &mut values);
            super::add_for_sql_update(update.login, "login", &mut columns, &mut values);
            super::add_for_sql_update(update.password, "password", &mut columns, &mut values);
            super::add_for_sql_update(update.settings, "preferences", &mut columns, &mut values);
            super::add_for_sql_update(update.user_ref, "user_ref", &mut columns, &mut values);
            super::add_for_sql_update(update.refresh_token, "refreshtoken", &mut columns, &mut values);
            super::add_for_sql_update(update.expires, "expires", &mut columns, &mut values);

            if columns.len() > 0 {
                values.push(Box::new(credential_id));
                let update_sql = format!("UPDATE Credentials SET {} WHERE id = ?", columns.join(", "));
                conn.execute(&update_sql, params_from_iter(values))?;
            }
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn add_crendential(&self, credential: Credential) -> Result<()> {
        self.server_store.call( move |conn| { 

            conn.execute("INSERT INTO Credentials (id, name, source, type, login, password, preferences, user_ref, refreshtoken, expires)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?)", params![
                credential.id,
                credential.name,
                credential.source,
                credential.kind,
                credential.login,
                credential.password,
                credential.settings,
                credential.user_ref,
                credential.refresh_token,
                credential.expires
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_credential(&self, credential_id: String) -> Result<()> {
        self.server_store.call( move |conn| { 
            conn.execute("DELETE FROM Credentials WHERE id = ?", &[&credential_id])?;
            Ok(())
        }).await?;
        Ok(())
    }
}