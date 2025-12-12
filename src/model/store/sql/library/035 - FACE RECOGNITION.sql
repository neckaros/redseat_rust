-- Drop old table (data loss acceptable as confirmed)
DROP TABLE IF EXISTS people_faces;

-- Assigned faces linked to identified people
CREATE TABLE people_faces (
    id TEXT PRIMARY KEY,
    people_ref TEXT NOT NULL,
    embedding BLOB NOT NULL,  -- Raw little-endian f32 (512 × 4 bytes = 2048 bytes)
    media_ref TEXT,
    bbox TEXT,  -- JSON: {"x1": f32, "y1": f32, "x2": f32, "y2": f32}
    confidence REAL,
    pose TEXT,  -- JSON: {"pitch": f32, "yaw": f32, "roll": f32} (optional)
    created INTEGER,
    FOREIGN KEY (people_ref) REFERENCES people(id),
    FOREIGN KEY (media_ref) REFERENCES medias(id)
);
CREATE INDEX idx_people_faces_person ON people_faces(people_ref);
CREATE INDEX idx_people_faces_media ON people_faces(media_ref);

-- Staging area for unassigned faces
CREATE TABLE unassigned_faces (
    id TEXT PRIMARY KEY,
    embedding BLOB NOT NULL,  -- Raw little-endian f32 (512 × 4 bytes = 2048 bytes)
    media_ref TEXT NOT NULL,
    bbox TEXT NOT NULL,
    confidence REAL NOT NULL,
    pose TEXT,  -- JSON: {"pitch": f32, "yaw": f32, "roll": f32}
    cluster_id TEXT,  -- Set by clustering algorithm
    created INTEGER NOT NULL,
    processed INTEGER DEFAULT 0,  -- Boolean: considered for clustering
    FOREIGN KEY (media_ref) REFERENCES medias(id)
);
CREATE INDEX idx_unassigned_faces_media ON unassigned_faces(media_ref);
CREATE INDEX idx_unassigned_faces_cluster ON unassigned_faces(cluster_id);
CREATE INDEX idx_unassigned_faces_created ON unassigned_faces(created);
CREATE INDEX idx_unassigned_faces_processed ON unassigned_faces(processed);

-- Track face processing status
ALTER TABLE medias ADD COLUMN face_processed INTEGER DEFAULT 0;

