CREATE TABLE scenarios (
                           id UUID PRIMARY KEY,
                           original_filename TEXT NOT NULL,
                           uploaded_at TIMESTAMPTZ NOT NULL,
                           file_size BIGINT NOT NULL CHECK (file_size >= 0),
                           parser_version TEXT NOT NULL,
                           game_version TEXT NOT NULL,
                           scenario_version TEXT NOT NULL
);

CREATE INDEX scenarios_uploaded_at_idx
    ON scenarios (uploaded_at DESC);