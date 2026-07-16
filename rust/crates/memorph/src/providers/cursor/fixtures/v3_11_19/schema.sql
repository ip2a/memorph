CREATE TABLE ItemTable (
    key TEXT UNIQUE ON CONFLICT REPLACE,
    value BLOB
);

CREATE TABLE cursorDiskKV (
    key TEXT UNIQUE ON CONFLICT REPLACE,
    value BLOB
);

CREATE TABLE composerHeaders (
    composerId TEXT PRIMARY KEY,
    workspaceId TEXT,
    createdAt INTEGER,
    lastUpdatedAt INTEGER,
    isArchived INTEGER,
    isSubagent INTEGER,
    recency INTEGER,
    checkpointAt INTEGER,
    value TEXT
);
