# Backup downloads

`GET /backups/:id/medias/db` downloads the latest database backup.
Library backups default to `library.rslibrary`, a Redseat restore package
containing library settings followed by the SQLite database.

Add `?sqlite=true` to download just the database as `database.sqlite`
(`application/vnd.sqlite3`). If the URL already has a token query parameter,
append `&sqlite=true`. Omitting the parameter or using `sqlite=false` preserves
the default download format.

The SQLite option also works for server database backups, which already contain
plain SQLite. It is ignored for media files other than `db`. Downloads use the
existing backup permissions and decryption. SQLite extraction streams the result
and does not support byte ranges or change the stored backup.

Use the `.rslibrary` package when importing a library back into Redseat; the SQLite
download does not include the library settings required by that importer.
