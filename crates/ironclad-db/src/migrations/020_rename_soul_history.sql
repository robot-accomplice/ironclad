-- Migration 020: Rename soul_history → os_personality_history
--
-- Part of the firmware/OS terminology coherency cleanup (v0.9.x).
-- "soul" was ambiguous — it historically meant the OS personality layer,
-- not firmware.  The new name makes this explicit.
--
-- SQLite supports ALTER TABLE … RENAME TO since 3.25.0 (2018-09-15).
ALTER TABLE soul_history RENAME TO os_personality_history;
