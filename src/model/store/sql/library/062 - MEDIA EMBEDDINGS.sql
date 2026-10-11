-- One vector per media and embedder. Vectors of different embedders are never comparable.
CREATE TABLE media_embeddings (
    media_ref TEXT NOT NULL,
    embedder TEXT NOT NULL,  -- e.g. embeddinggemma-2/q4/t140
    embedding BLOB NOT NULL, -- int8, one byte per value
    PRIMARY KEY (media_ref, embedder)
) WITHOUT ROWID;

-- Failed attempts, so a media that keeps failing stops being retried.
CREATE TABLE media_embedding_errors (
    media_ref TEXT NOT NULL,
    embedder TEXT NOT NULL,
    error TEXT,
    attempts INTEGER NOT NULL DEFAULT 1,
    modified INTEGER NOT NULL,
    PRIMARY KEY (media_ref, embedder)
) WITHOUT ROWID;

CREATE TRIGGER cleanup_media_embeddings AFTER DELETE ON medias BEGIN
    DELETE FROM media_embeddings WHERE media_ref = OLD.id;
    DELETE FROM media_embedding_errors WHERE media_ref = OLD.id;
END;
