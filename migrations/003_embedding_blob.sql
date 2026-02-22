-- Add binary BLOB storage for embeddings (~4x more compact than JSON TEXT).
-- Existing rows retain embedding_json for backward compatibility.
ALTER TABLE embeddings ADD COLUMN embedding_blob BLOB;
ALTER TABLE embeddings ADD COLUMN dimensions INTEGER NOT NULL DEFAULT 0;
