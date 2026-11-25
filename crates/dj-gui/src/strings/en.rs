//! dj-gui's text in English: every fixed label, button, menu item, title and tooltip.
//! A translation is a file like this one with the same names (see mod.rs).

// ---- Agents (MCP) ----

pub(crate) const WORK_WITH_AI_AGENTS_RUNNING_TIP: &str = "Work with AI agents running `dj mcp` on this project: show their drafts, follow their sessions, share your view. Saved for this project on this machine.";
pub(crate) const SHOW_AGENT_DRAFTS_TIP: &str = "Drafts in .local/drafts/ appear in the tree as unsaved segments, marked \"agent\"";
pub(crate) const SHOW_AGENT_DRAFTS: &str = "Show agent drafts";
pub(crate) const FOLLOW_AN_AGENT_SESSION: &str = "Follow an agent session";
pub(crate) const NO_AGENT_SESSIONS_DJ_MCP: &str = "No agent sessions (dj mcp)";
pub(crate) const STOP_FOLLOWING: &str = "Stop following";
pub(crate) const AGENTS_CAN_ASK_WHAT_YOU_TIP: &str = "Agents can ask what you are looking at (the gui_view tool) and open a session there to take it from here";
pub(crate) const MCP_ON_AGENT_DRAFTS_AND: &str = "MCP on: agent drafts and sessions are shown";
pub(crate) const MCP_OFF: &str = "MCP off";
pub(crate) const THE_AGENT_SESSION_YOU_FOLLOWED: &str = "The agent session you followed has closed.";
pub(crate) const AGENTS_DRAFTS_AND_SESSIONS_ARE_TIP: &str = "Agents' drafts and sessions are hidden. MCP > Enable MCP to show them.";
pub(crate) const ENABLE_MCP: &str = "Enable MCP";
pub(crate) const SHARE_MY_VIEW_WITH_AGENTS: &str = "Share my view with agents";
pub(crate) const MCP_NO_SESSIONS: &str = "MCP: no sessions";
pub(crate) const MCP_1_SESSION: &str = "MCP: 1 session";
pub(crate) const SHARING_VIEW: &str = " · sharing view";
pub(crate) const AGENTS_CAN_READ_WHAT_YOU: &str = " Agents can read what you are looking at (gui_view).";

// ---- Comparison cores ----

pub(crate) const RUN_COMPARISON_OFF_PAUSES_THE_TIP: &str = "Run comparison: off pauses the comparison core without unloading it; on syncs it again";
pub(crate) const SYNC_NOW: &str = "Sync now";
pub(crate) const BRING_THE_COMPARISON_CORE_TO_TIP: &str = "Bring the comparison core to the baseline's state at the frame shown";
pub(crate) const RE_RUN_EVERY_FAILURE_FROM_TIP: &str = "Re-run every failure from an older build: those that pass now are cleared";
pub(crate) const LOAD_A_COMPARISON_CORE_FIRST: &str = "Load a comparison core first";
pub(crate) const RUN: &str = "Run";
pub(crate) const RUN_IS_OFF: &str = "Run is off";
pub(crate) const BUSY_PAUSE_PLAYBACK_AND_WAIT: &str = "Busy: pause playback and wait for the clean or seek to finish";
pub(crate) const BY_COMPARING_SYSTEM_RAM: &str = "by comparing system RAM";
pub(crate) const AS_THE_CORE_JUDGES_IT: &str = "as the core judges it (dj_handle_event)";
pub(crate) const CHECK_PARITY: &str = "Check parity";
pub(crate) const NO_FAILURES_FROM_AN_OLDER: &str = "No failures from an older build";
pub(crate) const NONE: &str = "None";
pub(crate) const THE_PROJECT_HAS_NO_COMPARE: &str = "The project has no [[compare]] entries";

// ---- The debug console ----

pub(crate) const DEBUG_CONSOLE: &str = "Debug console";
pub(crate) const FILTER: &str = "Filter";
pub(crate) const COPY_TIP: &str = "Copy the lines shown, for a bug report";
pub(crate) const COPY: &str = "Copy";
pub(crate) const CLEAR: &str = "Clear";
pub(crate) const ERRORS: &str = "Errors";
pub(crate) const ERROR: &str = "Error";
pub(crate) const WARNINGS: &str = "Warnings";
pub(crate) const WARN: &str = "Warn";
pub(crate) const NOTES_AND_MESSAGES: &str = "Notes and messages";
pub(crate) const INFO: &str = "Info";

// ---- The Functions tab and the coverage map ----

pub(crate) const LOAD_A_COMPARISON_CORE_COMPARE: &str = "Load a comparison core (Compare > Comparison core) and turn on Check parity to see which code matches.";
pub(crate) const NO_NEW_CODE_IN_THIS: &str = "No new code in this segment";
pub(crate) const NO_SYMBOL_MAP_ADD_ONE: &str = "No symbol map: add one to see functions and blocks";
pub(crate) const DJ_ROLLS_COVERAGE_UP_INTO: &str = "dj rolls coverage up into the functions your symbols name; it doesn't find functions itself. Until then the map and the totals count the opcode bytes that ran.";
pub(crate) const IN_CONFIG_TOML_OR_DJ: &str = "In config.toml (or dj init --symbols FILE for a new project):";
pub(crate) const SYMBOLS_FILE_SYMBOLS_GAME_DBG: &str = "[[symbols]]\nfile = \"symbols/game.dbg\"\nformat = \"ca65-dbg\"";
pub(crate) const MORE_IN_DOCS_COVERAGE_MD: &str = "More in docs/coverage.md, Symbol maps.";
pub(crate) const ADD_SYMBOL_MAP: &str = "Add symbol map…";
pub(crate) const ADD_SYMBOL_MAP_TIP: &str = "Pick a symbol map: its format comes from the name, and a [[symbols]] entry is added to config.toml. A file outside the project is copied into symbols/ first.";
pub(crate) const BUSY_WAIT_FOR_THE_CLEAN_TIP: &str = "Busy: wait for the clean or seek to finish";
pub(crate) const COVERAGE_UPDATES_WHEN_THE_CLEAN: &str = "Coverage updates when the clean finishes.";
pub(crate) const NEW_CODE_RUN_BY_UNSAVED_TIP: &str = "New code run by unsaved segments (drafts too) or by the segment being played, and by no saved clean segment. It counts once saved and clean.";
pub(crate) const CODE_ONLY_DIRTY_SEGMENTS_RAN_TIP: &str = "Code only dirty segments ran, the last time each played. It counts once they are clean again (Project > Clean all segments), if they still run it.";
pub(crate) const ONLY_SAVED_SEGMENTS_THAT_AREN_TIP: &str = "Only saved segments that aren't dirty and whose current content is in the local database count, as in `dj summary`.";
pub(crate) const INSIDE_ANNOTATIONS_TOML_S_UNREACHABLE_TIP: &str = "Inside annotations.toml's [[unreachable]] ranges, left out of the counts";
pub(crate) const INSIDE_ANNOTATIONS_TOML_S_DATA_TIP: &str = "Inside annotations.toml's [[data]] ranges: never read as code or counted";
pub(crate) const NO_SYMBOL_MAP_COUNTING_OPCODE: &str = "No symbol map: counting opcode bytes";
pub(crate) const NO_CURRENT_SEGMENT_RUNS_THIS: &str = "No current segment runs this yet.";
pub(crate) const FILTER_BY_SEGMENT_TITLE: &str = "Filter by segment title";
pub(crate) const NO_SEGMENT_TITLE_MATCHES: &str = "No segment title matches.";
pub(crate) const ROM_ORDER: &str = "ROM order";
pub(crate) const MOST_UNCOVERED_TIP: &str = "Most instructions left to cover first";
pub(crate) const MOST_UNCOVERED: &str = "Most uncovered";
pub(crate) const LEAST_COVERED_TIP: &str = "Lowest percentage first";
pub(crate) const LEAST_COVERED: &str = "Least covered";
pub(crate) const NAME: &str = "Name";
pub(crate) const FILTER_BY_NAME: &str = "Filter by name";
pub(crate) const UNREACHABLE: &str = "unreachable";
pub(crate) const DATA: &str = "data";
pub(crate) const COVERAGE_NEEDS_THE_LOCAL_DATABASE: &str = "Coverage needs the local database (Project > Use the local database).";
pub(crate) const NO_COVERAGE_RECORDED_YET_CLEAN: &str = "No coverage recorded yet. Clean the project (Project > Clean all segments) to record which code each segment runs.";
pub(crate) const FAILED_OLDER_BUILD: &str = "failed, older build";
pub(crate) const FAILED: &str = "failed";
pub(crate) const LEGEND_PARITY: &str = "parity";
pub(crate) const NOT_CHECKED: &str = "not checked";
pub(crate) const NEVER_RUN: &str = "never run";
pub(crate) const COVERED: &str = "covered";
pub(crate) const NEW_NOT_COUNTED_YET: &str = "new, not counted yet";
pub(crate) const DIRTY_ONLY: &str = "dirty only";
pub(crate) const NOT_COUNTED_YET: &str = "not counted yet";
pub(crate) const COUNTED_ONCE_SAVED: &str = "counted once saved";
pub(crate) const COUNTED_ONCE_SAVED_AND_CLEAN: &str = "counted once saved and clean";
pub(crate) const COUNTED_ONCE_THE_SEGMENT_IS: &str = "counted once the segment is clean";
pub(crate) const NOT_COUNTED_NO_FULL_PASS: &str = "not counted: no full pass of its current content is recorded";
pub(crate) const HIDE_THE_MAP: &str = "Hide the map";
pub(crate) const HIDE: &str = "Hide";
pub(crate) const COVERAGE_WITH_PARITY_S_PURPLE: &str = "Coverage, with parity's purple, orange and red over it";
pub(crate) const COMBINED: &str = "Combined";
pub(crate) const COVERAGE_ONLY: &str = "Coverage only";
pub(crate) const COVERAGE: &str = "Coverage";
pub(crate) const PARITY_WITH_THE_COMPARISON_CORE: &str = "Parity with the comparison core only";
pub(crate) const MAP_PARITY: &str = "Parity";
pub(crate) const NO_COVERAGE_YET: &str = "No coverage yet.";
pub(crate) const CC65_S_DBG_LD65_DBGFILE: &str = "cc65's .dbg (ld65 --dbgfile)";
pub(crate) const A_MESEN_LABEL_FILE_MLB: &str = "a Mesen label file (.mlb), saved as you name code in Mesen's debugger";
pub(crate) const A_DECOMP_S_SYMBOL_ADDRS: &str = "a decomp's symbol_addrs.txt";
pub(crate) const A_GNU_LD_MAP_MAP: &str = "a GNU ld map (-Map=) from building the decomp";
pub(crate) const THE_ELF_THE_DECOMP_BUILDS: &str = "the ELF the decomp builds";
pub(crate) const NAME_REGION_OFFSET_SIZE_CPU: &str = "name,region,offset,size[,cpu_addr] in hex, from anything else (a Ghidra export, say)";
pub(crate) const CHOOSE_A_SYMBOL_MAP: &str = "Choose a symbol map";
pub(crate) const COPIED_INTO_THE_PROJECT: &str = ", copied into the project";
pub(crate) const COVERAGE_NEEDS_THE_ROM_AND: &str = "Coverage needs the ROM and the core.";
pub(crate) const FUNCTIONS: &str = "Functions";
pub(crate) const BLOCKS: &str = "Blocks";
pub(crate) const OPCODE_BYTES: &str = "Opcode bytes";
pub(crate) const INSTRUCTIONS: &str = "Instructions";
pub(crate) const DIRTY: &str = "  (dirty)";
pub(crate) const RUNS_IN: &str = "Runs in ";

// ---- Dialogs ----

pub(crate) const COMPARISON_CORE_TITLE: &str = "Comparison core";
pub(crate) const ONLY_THE_BASELINE_IS_SHOWN: &str = "Only the baseline is shown. The entries are [[compare]] in config.toml, or .local/local.toml for this machine.";
pub(crate) const OK: &str = "OK";
pub(crate) const UNSAVED_EDITS_TITLE: &str = "Unsaved edits";
pub(crate) const CANCEL: &str = "Cancel";
pub(crate) const CREATE: &str = "Create";
pub(crate) const SAVE_TITLE: &str = "Save";
pub(crate) const DIRTY_SEGMENTS_ARE_UNRECORDED_NO: &str = "Dirty segments are unrecorded (no recorded ending yet) or stale (they start where their parent no longer ends). Cleaning plays them and records their hashes; saved dirty, they stay dirty until cleaned and saved again.";
pub(crate) const REVIEW_THEM_BEFORE_SAVING_SAVING: &str = "Review them before saving: saving clears the marks.";
pub(crate) const CLEAN_THEN_SAVE: &str = "Clean, then save";
pub(crate) const SAVE_ANYWAY: &str = "Save anyway";
pub(crate) const REVIEW_FIRST: &str = "Review first";
pub(crate) const DUPLICATE_SEGMENT_TITLE: &str = "Duplicate segment";
pub(crate) const WITH_CHILDREN_TIP: &str = "Copy the whole branch: every segment below this one comes along, under the copy";
pub(crate) const WITH_CHILDREN: &str = "With children";
pub(crate) const JUST_THIS_SEGMENT_TIP: &str = "Copy only this segment; the copy has no children";
pub(crate) const JUST_THIS_SEGMENT: &str = "Just this segment";
pub(crate) const DELETE_SEGMENT_TITLE: &str = "Delete segment";
pub(crate) const DELETE_KEEP_CHILDREN: &str = "Delete, keep children";
pub(crate) const DELETE_KEEP_CHILDREN_TIP: &str = "Delete only this segment: its children move up to its parent (or become root segments) and turn dirty";
pub(crate) const JOIN_CHAIN_TITLE: &str = "Join chain";
pub(crate) const LEAVE_A_MARKER_WITH_EACH: &str = "Leave a marker with each segment's title where it began";
pub(crate) const JOIN: &str = "Join";
pub(crate) const JOIN_WITH_PARENT_TITLE: &str = "Join with parent";
pub(crate) const WHAT_HAPPENS_HERE: &str = "what happens here";
pub(crate) const SEGMENT_PROPERTIES_TITLE: &str = "Segment properties";
pub(crate) const TITLE: &str = "Title";
pub(crate) const TAGS: &str = "Tags";
pub(crate) const SPACE_SEPARATED_E_G_CHAPTER: &str = "space-separated, e.g. chapter-2 town";
pub(crate) const FREE_TAGS_SEPARATED_BY_SPACES_TIP: &str = "Free tags, separated by spaces. Filter the tree by one with #tag in the filter box, or the tag menu beside it";
pub(crate) const FRAMES: &str = "Frames";
pub(crate) const CHANGING_THE_SNAPSHOT_OR_SAVE: &str = "Changing the save changes how the segment starts: it and everything below turn dirty.";
pub(crate) const APPLY: &str = "Apply";
pub(crate) const EVENTS_RAM_WRITES_AND_OTHER: &str = "Events, RAM writes and other lines on this frame. Edit the text after the frame number.";
pub(crate) const NO_LINES_ON_THIS_FRAME: &str = "No lines on this frame yet.";
pub(crate) const DELETE: &str = "Delete";
pub(crate) const NEW: &str = "New:";
pub(crate) const EVENT: &str = "event";
pub(crate) const RAM_WRITE: &str = "RAM write";
pub(crate) const EVENT_BOSS_APPEARS: &str = "[EVENT] Boss appears";
pub(crate) const ADD: &str = "Add";
pub(crate) const A_WRITE_TAKES_ANY_NUMBER: &str = "A write takes any number of bytes: [WRITE] RAM 0x0300 01 02 03 …";
pub(crate) const CHOOSE_A_SNAPSHOT_SAVE_STATE: &str = "Choose a snapshot (save state)";
pub(crate) const A_SAVE_STATE_OF_THIS: &str = "A save state of this core to start from instead of power-on. It is copied into blobs/ under a name you can change.";
pub(crate) const NONE_POWER_ON: &str = "none (power-on)";
pub(crate) const SNAPSHOT: &str = "Snapshot";
pub(crate) const CHOOSE_A_SAVE_FILE_SRAM: &str = "Choose a save file (SRAM)";
pub(crate) const BATTERY_BACKED_SAVE_RAM_TO: &str = "Battery-backed save RAM to load before frame 0 (an .srm, or the core's save file), after the snapshot if there is one. It is copied into blobs/ under a name you can change.";
pub(crate) const SAVE_FILE: &str = "Save file";

// ---- Editing ----

pub(crate) const NOTHING_TO_PASTE_HERE_THE: &str = "Nothing to paste here: the play cursor is past the copied notes' room";
pub(crate) const PASTE: &str = "Paste";

// ---- Background work ----

pub(crate) const NO_PRE_ROLL_THE_SEGMENT: &str = "No pre-roll: the segment above no longer ends where this one starts.";
pub(crate) const CLEANING_NEEDS_THE_LOCAL_DATABASE: &str = "Cleaning needs the local database (Project > Use the local database).";
pub(crate) const ALL_SEGMENTS: &str = "all segments";
pub(crate) const UNSAVED_SAVE_TO_KEEP_THEM: &str = ". Unsaved: save to keep them, or undo";
pub(crate) const YOU_STOPPED_IT: &str = "you stopped it";

// ---- The window, and what several panels share ----

pub(crate) const OPEN: &str = "Open";
pub(crate) const RELOAD_ALL: &str = "Reload all";
pub(crate) const QUIT: &str = "Quit";
pub(crate) const STOP: &str = "Stop";
pub(crate) const LOOP: &str = "Loop";
pub(crate) const NEXT: &str = "Next";
pub(crate) const LIST: &str = "List";
pub(crate) const STOP_AT_THE_END_OF: &str = "Stop at the end of each segment.";
pub(crate) const PLAY_THE_SEGMENT_AGAIN_FROM: &str = "Play the segment again from its first frame, until you stop it.";
pub(crate) const CARRY_ON_INTO_THE_NEXT: &str = "Carry on into the next segment: the child you came back up from (sticky), or the only child. Stops where a segment has several children and none is sticky, or where the child is open in an agent session.";
pub(crate) const CARRY_ON_DOWN_THE_SEGMENT: &str = "Carry on down the segment list in tree order, through every segment below and after this one (whether folded or filtered out), to the end of the list. With the tree focused on a segment (Focus here), it stops at the end of that segment's branch. Started on a root segment, it plays the whole tree.";
pub(crate) const NEW_ROOT_TIP: &str = "A new empty segment with no parent: it starts from power-on (or from a save file set in its properties)";
pub(crate) const JOIN_CHAIN_TIP: &str = "Join every only-child below this segment into it, down to the first segment with no children or several, as one undo step";
pub(crate) const DUP_TIP: &str = "A copy of this segment beside it (same parent, same inputs); with children, it asks whether to copy them too";
pub(crate) const MOVE_TIP: &str = "Give this segment (and what is below it) another parent, or make it a root segment. It and everything below turn dirty.";
pub(crate) const JOIN_TIP: &str = "Merge this segment into its parent: the parent keeps its name and grows by this segment's frames. Only for a segment with no siblings.";
pub(crate) const JOIN_CHILD_TIP: &str = "Merge this segment's only child into it: this segment keeps its name and grows by the child's frames. Only for a segment with exactly one child.";
pub(crate) const SELECT_A_SEGMENT_TO_SEE: &str = "Select a segment to see its inputs.";
pub(crate) const OPEN_THE_OTHER_PROJECT: &str = "open the other project";
pub(crate) const RELOAD_EVERY_SEGMENT_FROM_DISK: &str = "reload every segment from disk";
pub(crate) const DISCARD_QUIT: &str = "quit";
pub(crate) const MAX: &str = "MAX";
pub(crate) const EDIT_NOTES: &str = "Edit notes";
pub(crate) const EDIT_STICK: &str = "Edit stick";
pub(crate) const STICK_PAD: &str = "Stick pad";
pub(crate) const STICK_PAD_TITLE: &str = "Stick pad";
pub(crate) const STICK_PAD_TIP: &str = "A square to drag a stick's position in: sets X and Y together on the frame shown.";
pub(crate) const LEFT_STICK: &str = "Left stick";
pub(crate) const RIGHT_STICK: &str = "Right stick";
pub(crate) const THIS_STICK_IS_BUTTONS_TIP: &str = "The project shows this stick as buttons: set them in the piano roll.";
pub(crate) const CENTRE: &str = "Centre";
pub(crate) const CENTRE_TIP: &str = "Let the stick go on this frame.";
pub(crate) const HOLD_WHILE_STEPPING: &str = "Hold while stepping";
pub(crate) const HOLD_WHILE_STEPPING_TIP: &str = "Stepping forward puts the stick in the same position on each frame stepped onto. Playing or jumping to a frame doesn't.";
pub(crate) const THIS_SEGMENT_IS_READ_ONLY: &str = "This segment is read-only (an agent has it open).";
pub(crate) const SELECT_A_SEGMENT_TO_SET: &str = "Select a segment to set its stick.";
pub(crate) const STICK_LEFT: &str = "left";
pub(crate) const STICK_RIGHT: &str = "right";
pub(crate) const STICK_UP: &str = "up";
pub(crate) const STICK_DOWN: &str = "down";

// ---- The menu bar ----

pub(crate) const FILE_MENU: &str = "File";
pub(crate) const OPEN_PROJECT: &str = "Open project…";
pub(crate) const CTRL_S_TIP: &str = "Write every changed segment, and move drafts into segments/";
pub(crate) const WRITE_ONLY_THIS_SEGMENT_S_TIP: &str = "Write only this segment's file; other unsaved edits stay unsaved";
pub(crate) const SAVE_SELECTED_SEGMENT: &str = "Save selected segment";
pub(crate) const REVERT_SELECTED_SEGMENT_TIP: &str = "Drop this segment's unsaved changes and put back its saved file; other edits are kept. Undo brings the changes back";
pub(crate) const REVERT_SELECTED_SEGMENT: &str = "Revert selected segment";
pub(crate) const RELOAD_ALL_FROM_DISK_TIP: &str = "Read every segment's file again, dropping the unsaved edits of every segment (it asks first when there are any). To drop one segment's edits, use Revert selected segment";
pub(crate) const RELOAD_ALL_FROM_DISK: &str = "Reload all from disk";
pub(crate) const COMING_YOUR_GUI_PREFERENCES_FROM_TIP: &str = "Coming: your GUI preferences from .local/local.toml in one window";
pub(crate) const OPTIONS: &str = "Options…";
pub(crate) const KEYBOARD_MAPPING_TIP: &str = "Keys for recording inputs from the keyboard";
pub(crate) const KEYBOARD_MAPPING: &str = "Keyboard mapping…";
pub(crate) const EDIT_MENU: &str = "Edit";
pub(crate) const CUT: &str = "Cut";
pub(crate) const PASTE_AT_PLAY_CURSOR: &str = "Paste at play cursor";
pub(crate) const SELECT_EVERY_PRESS_OF_THE_TIP: &str = "Select every press of the selected segment in the piano roll";
pub(crate) const SELECT_ALL: &str = "Select all";
pub(crate) const DRAG_TO_DRAW_INPUTS: &str = "Drag to draw inputs";
pub(crate) const NEW_CHILD_AT_END_TIP: &str = "A new empty segment that starts where this one ends";
pub(crate) const NEW_CHILD_AT_END: &str = "New child at end";
pub(crate) const NEW_ROOT_SEGMENT: &str = "New root segment";
pub(crate) const NEW_ROOT_SEGMENT_FROM_HERE: &str = "New root segment from here";
pub(crate) const NEW_ROOT_SEGMENT_FROM_HERE_TIP: &str = "A new segment with no parent that starts where the playhead is: a snapshot of the state after the frame shown, with the inputs held there. The snapshot is a file in blobs/";
pub(crate) const THIS_SAVES_A_SNAPSHOT_OF: &str = "This saves a snapshot of the frame shown into the project's blobs folder.";
pub(crate) const DON_T_SHOW_THIS_AGAIN: &str = "Don't show this again";
pub(crate) const DON_T_SHOW_THIS_AGAIN_TIP: &str = "Kept for you in this project's .local/local.toml";
pub(crate) const STILL_GETTING_TO_THE_FRAME: &str = "still getting to that frame: try again when it is shown";
pub(crate) const DUPLICATE: &str = "Duplicate";
pub(crate) const BRANCH_AT_PLAYHEAD_TIP: &str = "Split this segment after the playhead and start a second path there";
pub(crate) const BRANCH_AT_PLAYHEAD: &str = "Branch at playhead";
pub(crate) const SPLIT_AT_PLAYHEAD_TIP: &str = "Cut this segment in two after the playhead; its children move to the second half";
pub(crate) const SPLIT_AT_PLAYHEAD: &str = "Split at playhead";
pub(crate) const ADD_EVENT_AT_PLAYHEAD: &str = "Add event at playhead…";
pub(crate) const PROPERTIES: &str = "Properties…";
pub(crate) const ONLY_FOR_A_SEGMENT_WHOSE_TIP: &str = "Only for a segment whose parent has no other children";
pub(crate) const ONLY_FOR_A_SEGMENT_WITH_TIP: &str = "Only for a segment with exactly one child";
pub(crate) const JOIN_WITH_ONLY_CHILD: &str = "Join with only child";
pub(crate) const ONLY_WHERE_AT_LEAST_TWO_TIP: &str = "Only where at least two only-children follow in a row";
pub(crate) const JOIN_CHAIN: &str = "Join chain…";
pub(crate) const MOVE_TO: &str = "Move to…";
pub(crate) const DELETE_TIP: &str = "Delete this segment and everything below it, or just it, keeping its children";
pub(crate) const DELETE_MENU_ITEM: &str = "Delete…";
pub(crate) const VIEW_MENU: &str = "View";
pub(crate) const ZOOM_IN: &str = "Zoom in";
pub(crate) const ZOOM_OUT: &str = "Zoom out";
pub(crate) const FOLLOW_PLAYHEAD: &str = "Follow playhead";
pub(crate) const RETURN_TO_THE_EDIT_TIP: &str = "Any edit pauses playback and shows the earliest frame edited since you last played or moved the playhead. Off: the frame you were on is redrawn.";
pub(crate) const RETURN_TO_THE_EDIT: &str = "Return to the edit";
pub(crate) const SHOW_EVERY_JOYPAD_BUTTON: &str = "Show every joypad button";
pub(crate) const A_LOG_BELOW_THE_COVERAGE_TIP: &str = "A log, below the coverage map, of what dj says while it runs: warnings and notes, the cores' own log messages, what comparison cores and decoder plugins print, and the status bar's messages";
pub(crate) const PROJECT_MENU: &str = "Project";
pub(crate) const CLEAN_ALL_SEGMENTS: &str = "Clean all segments";
pub(crate) const CLEAN_ALL_SEGMENTS_TIP: &str = "Play, top down, every segment that isn't clean or whose content isn't in the local database yet, and record the hashes it reaches (as reaching a segment's end does). Changed endings are marked ! for you to check. Results are unsaved changes.";
pub(crate) const NEEDS_THE_LOCAL_DATABASE_TIP: &str = "Needs the local database";
pub(crate) const REPLAY_EVERYTHING: &str = "Replay everything";
pub(crate) const REPLAY_EVERYTHING_TIP: &str = "Play every segment again from frame 0 of the first root segment (or of the focused segment), in List mode, as ordinary playback you can pause, watch and speed up: a check that they still end as recorded (after a core update, say). Changed endings are marked !";
pub(crate) const USE_THE_LOCAL_DATABASE_TIP: &str = "Off: play from power-on and keep snapshots only in memory, as before the database existed";
pub(crate) const SET_ROM: &str = "Set ROM…";
pub(crate) const SET_CORE: &str = "Set core…";
pub(crate) const COMING_EDIT_THE_PROJECT_S_TIP: &str = "Coming: edit the project's config.toml and its local.toml settings here";
pub(crate) const PROJECT_SETTINGS: &str = "Project settings…";
pub(crate) const EMULATION_MENU: &str = "Emulation";
pub(crate) const STEP_FORWARD: &str = "Step forward";
pub(crate) const STEP_BACK: &str = "Step back";
pub(crate) const GO_TO_START: &str = "Go to start";
pub(crate) const GO_TO_END: &str = "Go to end";
pub(crate) const COMPARE_MENU: &str = "Compare";
pub(crate) const MCP_MENU: &str = "MCP";
pub(crate) const OPEN_A_PROJECT_FOLDER: &str = "Open a project folder";
pub(crate) const CTRL_S: &str = "Ctrl+S";
pub(crate) const NO_SEGMENT_SELECTED: &str = "No segment selected";
pub(crate) const UNDO: &str = "Undo";
pub(crate) const REDO: &str = "Redo";
pub(crate) const IN_THE_PIANO_ROLL_A: &str = "In the piano roll, a drag on an empty cell draws a press instead of selecting an area. Hold Ctrl/Cmd for the other one.";
pub(crate) const USE_THE_LOCAL_DATABASE: &str = "Use the local database";
pub(crate) const CHOOSE_THE_ROM: &str = "Choose the ROM";
pub(crate) const LIBRETRO_CORE: &str = "libretro core";
pub(crate) const CHOOSE_THE_LIBRETRO_CORE: &str = "Choose the libretro core";
pub(crate) const SPACE: &str = "Space";
pub(crate) const PLAY: &str = "Play";
pub(crate) const PAUSE: &str = "Pause";
pub(crate) const RIGHT_ARROW: &str = "Right arrow";
pub(crate) const LEFT_ARROW: &str = "Left arrow";
pub(crate) const HOME: &str = "Home";
pub(crate) const END: &str = "End";

// ---- Segment operations ----

pub(crate) const A_SEGMENT_CAN_T_MOVE: &str = "A segment can't move below itself.";
pub(crate) const MOVE_SEGMENT_TITLE: &str = "Move segment";
pub(crate) const IT_THEN_STARTS_WHERE_THE: &str = "It then starts where the new parent ends, so it and everything below turn dirty.";
pub(crate) const FILTER_BY_TITLE: &str = "Filter by title";
pub(crate) const NO_PARENT_A_ROOT_SEGMENT: &str = "No parent: a root segment, from power-on";
pub(crate) const ADDED_A_600_FRAME_ROOT: &str = "Added a 600-frame root segment that starts from power-on. Change its title, length and save file with Edit > Properties….";
pub(crate) const ADDED_A_600_FRAME_SEGMENT: &str = "Added a 600-frame segment. Change its title and length with Edit > Properties….";
pub(crate) const THIS_SEGMENT_HAS_NO_ONLY: &str = "this segment has no only child to join";
pub(crate) const NO_PROJECT: &str = "no project";
pub(crate) const ONLY_A_SEGMENT_WHOSE_PARENT: &str = "only a segment whose parent has no other children can be joined";
pub(crate) const TO_A_ROOT_SEGMENT_STARTING: &str = "to a root segment, starting from power-on";
pub(crate) const MOVE: &str = "Move";
pub(crate) const ITS_SAVE_FILE_OR_SNAPSHOT: &str = " Its save file or snapshot was dropped: only a root segment has one.";
pub(crate) const ARE_NOW_ROOT_SEGMENTS_STARTING: &str = "are now root segments, starting from power-on";
pub(crate) const MOVED_UP_TO_ITS_PARENT: &str = "moved up to its parent";
pub(crate) const DUPLICATED_THE_COPY_HAS_THE: &str = "Duplicated. The copy has the same inputs and hashes; its children weren't copied.";
pub(crate) const NEW_SEGMENT: &str = "New segment";
pub(crate) const SELECT_A_SEGMENT_FIRST: &str = "select a segment first";
pub(crate) const THE_PLAYHEAD_IS_ON_THE: &str = "the playhead is on the last frame: use New child at end instead";
pub(crate) const SPLIT: &str = "Split";
pub(crate) const BRANCH: &str = "Branch";
pub(crate) const NEW_BRANCH: &str = "New branch";
pub(crate) const SEGMENT_IS_GONE: &str = "segment is gone";
pub(crate) const A_SEGMENT_NEEDS_AT_LEAST: &str = "a segment needs at least one frame";
pub(crate) const SAVE_FILE_SET: &str = "save file set";
pub(crate) const SAVE_FILE_CLEARED: &str = "save file cleared";
pub(crate) const SNAPSHOT_SET: &str = "snapshot set";
pub(crate) const SNAPSHOT_CLEARED: &str = "snapshot cleared";
pub(crate) const EDIT_LINE: &str = "Edit line";
pub(crate) const ADD_LINE: &str = "Add line";
pub(crate) const DELETE_LINE: &str = "Delete line";

// ---- Parity ----

pub(crate) const STOPPED_RE_RUNNING_FAILURES: &str = "Stopped re-running failures";
pub(crate) const INSTRUCTIONS_FRAMES_THAT_PASSED_RAN_TIP: &str = "Instructions frames that passed ran, and no failing frame did";
pub(crate) const RUN_BY_A_FRAME_THAT_TIP: &str = "Run by a frame that failed with this build. A later pass doesn't clear it: clear the failure once it's fixed.";
pub(crate) const RE_RUN_THE_FAILURES_TO_TIP: &str = "Re-run the failures to check them with this build";
pub(crate) const RAN_BUT_NOT_IN_A_TIP: &str = "Ran, but not in a frame checked against this build";
pub(crate) const CLEAR_TIP: &str = "Forget this failure";
pub(crate) const RUNNING: &str = "running…";
pub(crate) const RE_RUN: &str = "Re-run";
pub(crate) const ONE_PASSES_NOW_CLEARED: &str = "1 passes now and was cleared";
pub(crate) const ONE_PASSED_STAYS_RED: &str = "1 passed but stays red (it failed with this build)";
pub(crate) const THIS_BUILD: &str = "this build";
pub(crate) const AN_OLDER_BUILD: &str = "an older build";

// ---- Playback ----

pub(crate) const RECORDING_IS_AT_THE_END: &str = "Recording is at the end of the segment: turn on Extend segment (Record menu) to record past it, or move the playhead back";
pub(crate) const NO_SEGMENTS_TO_REPLAY: &str = "No segments to replay";
pub(crate) const PLAY_OR_STEP_ON_TO: &str = "Play or step on to sync again.";
pub(crate) const PLAY_OR_STEP_ON_TO_CARRY: &str = "Play or step on to carry on from here; Compare > Sync now has the core copy the baseline again.";
pub(crate) const NO_SNAPSHOT: &str = "no snapshot";
pub(crate) const EVERY_SEGMENT: &str = "every segment";
pub(crate) const THE_FOCUSED_BRANCH: &str = "the focused branch";
pub(crate) const LIST_REACHED_THE_END_OF: &str = "List: reached the end of the segment list";
pub(crate) const LIST_REACHED_THE_END_OF_THE: &str = "List: reached the end of the focused branch";

// ---- The project ----

pub(crate) const CLEANING_THE_SEGMENT_IS_SHOWN: &str = "Cleaning: the segment is shown once the clean stops or finishes.";
pub(crate) const NO_ROM_SET_FOR_THIS: &str = "No ROM set for this project on this machine. Use Emulation > Set ROM…";
pub(crate) const NO_LIBRETRO_CORE_SET_FOR: &str = "No libretro core set for this project on this machine. Use Emulation > Set core…";
pub(crate) const RUNNING_WITHOUT_THE_LOCAL_DATABASE: &str = "Running without the local database";
pub(crate) const USING_THE_LOCAL_DATABASE: &str = "Using the local database";

// ---- New project ----

pub(crate) const NEW_PROJECT: &str = "New project…";
pub(crate) const NEW_PROJECT_TIP: &str = "Make a project from a game's ROM and a core, as dj init does";
pub(crate) const NEW_PROJECT_TITLE: &str = "New project";
pub(crate) const NEW_ROM_FILE: &str = "ROM file";
pub(crate) const NEW_ROM_HINT: &str = "The game's ROM";
pub(crate) const NEW_BROWSE: &str = "Browse…";
pub(crate) const NEW_FOLDER: &str = "Folder";
pub(crate) const NEW_FOLDER_TIP: &str = "The folder the project is made in: config.toml, segments/ and blobs/ go straight into it. A .dj folder is found by dj from the folder it is in and from any below.";
pub(crate) const NEW_FOLDER_BROWSE_TIP: &str = "The folder you choose becomes the project's own folder";
pub(crate) const NEW_NAME: &str = "Name";
pub(crate) const NEW_CORE: &str = "Core";
pub(crate) const NEW_OTHER_CORE: &str = "Other core file…";
pub(crate) const NEW_CORE_HINT: &str = "A libretro core's library";
pub(crate) const NEW_UNKNOWN_CONSOLE: &str = "dj can't tell which console this ROM is for (it knows NES, SNES, GBA and N64 ROMs), so it has no core to suggest. Any libretro core plays; one without a connector records no coverage.";
pub(crate) const NEW_SYMBOL_MAP: &str = "Symbol map";
pub(crate) const NEW_SYMBOL_MAP_HINT: &str = "None";
pub(crate) const NEW_SYMBOL_MAP_TIP: &str = "Optional: a cc65 .dbg, Mesen .mlb, GNU ld .map, ELF, CSV or splat symbol_addrs.txt, copied into the project's symbols/ folder. One can be added later from the Functions tab.";
pub(crate) const NEW_FIRST_SEGMENT: &str = "Make a first segment (60 frames from power-on)";
pub(crate) const NEW_FIRST_SEGMENT_TIP: &str = "A root segment to record into. Without it, make one with Edit > New root segment.";
pub(crate) const NEW_FIRST_SEGMENT_TITLE: &str = "Power-on";
pub(crate) const NEW_CHOOSE_THE_ROM: &str = "Choose the game's ROM file";
pub(crate) const NEW_CHOOSE_A_FOLDER: &str = "Choose the project's folder";
pub(crate) const NEW_CHOOSE_A_CORE: &str = "Choose a core";
pub(crate) const NEW_NAME_THE_PROJECT: &str = "Give the project a name";
pub(crate) const NEW_LOADING: &str = "Loading the core with the ROM…";
pub(crate) const NEW_CORE_DIDNT_LOAD: &str = "The core didn't load this ROM";
pub(crate) const NEW_CORE_LOADS: &str = "The core loads this ROM. The project would pin:";
pub(crate) const NEW_NOT_CHECKED: &str = "The core hasn't loaded this ROM yet";
pub(crate) const NEW_CREATE: &str = "Create project";
pub(crate) const NEW_WAIT_FOR_THE_JOB: &str = "A job is running: make a new project once it's done.";
pub(crate) const NEW_ROW_CONSOLE: &str = "Console";
pub(crate) const NEW_ROW_BUILD: &str = "Core build";
pub(crate) const NEW_ROW_COVERAGE: &str = "Coverage";
pub(crate) const NEW_ROW_HASHES: &str = "Hashes";
pub(crate) const NEW_ROW_SNAPSHOTS: &str = "Snapshots";
pub(crate) const NEW_ROW_CONTROLLER: &str = "Input";
pub(crate) const NEW_ROW_SYMBOLS: &str = "Symbols";
pub(crate) const NEW_NO_CONSOLE_DEFAULTS: &str = "No defaults for this core: a plain joypad, RAM hashed";
pub(crate) const NEW_NO_CONNECTOR: &str = "No: this core has no coverage connector";
pub(crate) const NEW_COVERAGE_NO_CODE_REGION: &str = "The connector names no code region";
pub(crate) const NEW_DEFAULT_PRESET: &str = "The default preset (nes-sparse)";

// ---- Keyboard mapping ----

pub(crate) const KEYBOARD_MAPPING_TITLE: &str = "Keyboard mapping";
pub(crate) const KEYS_FOR_THE_CONTROLLER_BEING: &str = "Keys for the controller being recorded. Click a key, then press the new one (Escape cancels).";
pub(crate) const NO_KEY_TIP: &str = "No key";
pub(crate) const RESET_TO_DEFAULT: &str = "Reset to default";
pub(crate) const COMMIT_FRAME_BY_FRAME: &str = "Commit (frame by frame)";
pub(crate) const REWIND_FRAME_BY_FRAME: &str = "Rewind (frame by frame)";
pub(crate) const PRESS_A_KEY: &str = "press a key…";

// ---- Recording ----

pub(crate) const AT_THE_END_OF_THE: &str = "At the end of the segment: turn on Extend segment (Record menu) to record past it";
pub(crate) const FRAME_BY_FRAME_TIP: &str = "Off: real time, Space plays and pauses and every played frame records what you hold. On: the commit key (Space) writes what you hold to the next frame and steps to it; rewind (Backspace) steps back";
pub(crate) const FRAME_BY_FRAME: &str = "Frame by frame";
pub(crate) const REWIND_CLEARS_INPUTS_TIP: &str = "Rewind also clears the input on each frame it leaves. Shift+rewind does the opposite";
pub(crate) const REWIND_CLEARS_INPUTS: &str = "Rewind clears inputs";
pub(crate) const OVERWRITE_ALL_INPUTS_TIP: &str = "Each recorded frame clears every controller's input first, so only what you hold is left. Off: held buttons are only added (to record player 2 over player 1, say)";
pub(crate) const OVERWRITE_ALL_INPUTS: &str = "Overwrite all inputs";
pub(crate) const EXTEND_SEGMENT_TIP: &str = "Recording past the last frame makes the segment longer. Off: playback pauses at the end and recording stays on";
pub(crate) const EXTEND_SEGMENT: &str = "Extend segment";
pub(crate) const RECORDING_OPTIONS_TIP: &str = "Recording options";
pub(crate) const SELECT_A_SEGMENT_TO_RECORD: &str = "Select a segment to record into";
pub(crate) const THIS_DRAFT_IS_OPEN_IN: &str = "This draft is open in an agent session and is read-only: right-click it and Take over to record into it";
pub(crate) const BUSY_WAIT_FOR_THE_SEGMENT: &str = "Busy: wait for the segment to load or the clean to finish";
pub(crate) const PLAY_TO_RECORD_WHAT_YOU: &str = "play to record what you hold";
pub(crate) const COMMIT_A_FRAME_WITH_THE: &str = "commit a frame with the commit key, step back with rewind";
pub(crate) const RECORD_INPUTS: &str = "Record inputs";
pub(crate) const RECORDING_CLICK_OR_PRESS_ESCAPE: &str = "Recording: click or press Escape to stop";

// ---- The piano roll ----

pub(crate) const INPUTS: &str = "Inputs";
pub(crate) const ALL_BUTTONS: &str = "all buttons";
pub(crate) const LABELS: &str = "labels";
pub(crate) const FOLLOW: &str = "follow";
pub(crate) const FIT_TIP: &str = "Fit the whole segment";
pub(crate) const FIT: &str = "fit";
pub(crate) const JOYPAD: &str = "Joypad";
pub(crate) const MOUSE: &str = "Mouse";
pub(crate) const KEYBOARD: &str = "Keyboard";
pub(crate) const LIGHTGUN: &str = "Lightgun";
pub(crate) const ANALOG: &str = "Analog";
pub(crate) const POINTER: &str = "Pointer";
pub(crate) const DEVICE: &str = "Device";
pub(crate) const RESET_LABEL: &str = "reset";
pub(crate) const NO_INPUTS_IN_THIS_SEGMENT: &str = "no inputs in this segment";
pub(crate) const FRAME: &str = "frame";
pub(crate) const WRITES_OTHER: &str = "writes, other";

// ---- Saving and recovery ----

pub(crate) const DJ_GUI_DIDN_T_CLOSE_TITLE: &str = "dj-gui didn't close normally";
pub(crate) const THE_LAST_DJ_GUI_TO: &str = "The last dj-gui to open this project ended without closing it (it crashed, or was quit by force).";
pub(crate) const RESTORE: &str = "Restore";
pub(crate) const DISCARD: &str = "Discard";
pub(crate) const RESTORE_UNSAVED_EDITS: &str = "Restore unsaved edits";
pub(crate) const NO_UNSAVED_CHANGES_IN_THIS: &str = "No unsaved changes in this segment";
pub(crate) const AN_AGENT_SESSION_HAS_THIS: &str = "An agent session has this draft open";
pub(crate) const ITS_PARENT_HAS_NEVER_BEEN: &str = "Its parent has never been saved: save that one first (or everything)";
pub(crate) const IT_IS_DIRTY_PLAY_IT: &str = " It is dirty: play it to its end (or clean it) and save again to keep its hashes.";
pub(crate) const NEVER_SAVED_SO_THERE_IS: &str = "Never saved, so there is no file to go back to (Delete removes it)";

// ---- The transport bar ----

pub(crate) const AT_THE_START_OF_A_TIP: &str = "At the start of a root segment";
pub(crate) const PREVIOUS_MARKER_IN_THIS_SEGMENT_TIP: &str = "Previous marker in this segment";
pub(crate) const NO_MARKER_BEFORE_THE_PLAYHEAD_TIP: &str = "No marker before the playhead in this segment";
pub(crate) const STEP_BACK_LEFT_ARROW_TIP: &str = "Step back (left arrow)";
pub(crate) const PLAY_PAUSE_SPACE_TIP: &str = "Play / pause (Space)";
pub(crate) const STEP_FORWARD_RIGHT_ARROW_TIP: &str = "Step forward (right arrow)";
pub(crate) const NEXT_MARKER_IN_THIS_SEGMENT_TIP: &str = "Next marker in this segment";
pub(crate) const NO_MARKER_AFTER_THE_PLAYHEAD_TIP: &str = "No marker after the playhead in this segment";
pub(crate) const AT_THE_END_OF_A_TIP: &str = "At the end of a segment with no children";
pub(crate) const PICK_A_CHILD_SEGMENT_TIP: &str = "Pick a child segment";
pub(crate) const THIS_SEGMENT_HAS_NO_CHILDREN_TIP: &str = "This segment has no children";
pub(crate) const STOP_AT_A_CHANGED_ENDING: &str = "Stop at a changed ending";
pub(crate) const STOP_AT_A_CHANGED_ENDING_TIP: &str = "Pause as soon as a segment records an ending that differs from its saved file (the !), so you can undo before the segments below it replay. Off: playback carries on, since a child always starts from where its parent really ends.";
pub(crate) const BEHAVIOR_AT_THE_END_OF_TIP: &str = "Behavior at the end of the segment";
pub(crate) const SLOWER_TIP: &str = "Slower";
pub(crate) const PLAYBACK_SPEED_TIP: &str = "Playback speed";
pub(crate) const FASTER_TIP: &str = "Faster";
pub(crate) const GO_TO_START_HOME_AT: &str = "Go to start (Home). At the start, this goes back to the end of the parent segment";
pub(crate) const GO_TO_END_END_AT: &str = "Go to end (End). At the end, this goes forward into the child segment";
pub(crate) const ENDING_CHANGED_SINCE_THE_LAST: &str = "! ending changed since the last save";
pub(crate) const CLEAN_DOT: &str = "● clean";
pub(crate) const DIRTY_WAITING_ON_A_SEGMENT: &str = "● dirty: waiting on a segment above";
pub(crate) const DIRTY_PLAY_TO_THE_END: &str = "● dirty: play to the end to record its hashes";
pub(crate) const DIRTY_PLAY_TO_THE_END_TO: &str = "● dirty: play to the end to re-check";

// ---- The tree of segments ----

pub(crate) const NO_PROJECT_OPEN: &str = "No project open.";
pub(crate) const UNSAVED_SEGMENTS_WHOSE_END_HASH_TIP: &str = "Unsaved segments whose end hash differs from their saved file. Save to keep them, or undo.";
pub(crate) const ALL_TIP: &str = "Show the whole tree";
pub(crate) const ALL: &str = "All";
pub(crate) const EXPAND_ALL_TIP: &str = "Expand all";
pub(crate) const COLLAPSE_ALL_TIP: &str = "Collapse all";
pub(crate) const NO_TAGS_YET_ADD_THEM: &str = "No tags yet. Add them in a segment's Properties.";
pub(crate) const SHOW_WHAT_S_BELOW_THE_TIP: &str = "While filtering, also show every segment below each match, not only the path down to it";
pub(crate) const SHOW_WHAT_S_BELOW_THE: &str = "Show what's below the matches";
pub(crate) const FILTER_BY_TAG_TIP: &str = "Filter by tag";
pub(crate) const CLEAR_THE_FILTER_TIP: &str = "Clear the filter";
pub(crate) const FILTER_TITLE_ID_TAG_OR: &str = "Filter: title, id, tag or #tag";
pub(crate) const ITS_ENDING_CHANGED_SINCE_IT_TIP: &str = "Its ending changed since it was last saved. Save to keep it, or undo.";
pub(crate) const STORED_THIS_SEGMENT_S_CURRENT_TIP: &str = "Stored: this segment's current content is in the local database";
pub(crate) const DB: &str = "db";
pub(crate) const HAS_WRITE_LINES_MEMORY_IS_TIP: &str = "Has [WRITE] lines: memory is changed directly";
pub(crate) const POKE: &str = "poke";
pub(crate) const EDITED_NOT_SAVED: &str = "edited, not saved";
pub(crate) const SHOW_THIS_DRAFT_AT_THE_TIP: &str = "Show this draft at the agent's frame as it plays";
pub(crate) const TAKE_OVER_TIP: &str = "Close the agent's session, keeping the draft, so it is yours to edit and save. The agent is told the next time it uses the session.";
pub(crate) const TAKE_OVER: &str = "Take over";
pub(crate) const PLAY_TO_HERE_TIP: &str = "Play through to the end of this segment";
pub(crate) const PLAY_TO_HERE: &str = "Play to here";
pub(crate) const REVERT_TIP: &str = "Drop this segment's unsaved changes and put back its saved file. Undo brings them back";
pub(crate) const REVERT: &str = "Revert";
pub(crate) const CLEAN_THIS_AND_BELOW: &str = "Clean this and below";
pub(crate) const CLEAN_THIS_AND_BELOW_TIP: &str = "Play what isn't clean or isn't in the local database here and below, and record the hashes it reaches";
pub(crate) const FOCUS_HERE_TIP: &str = "Show only this segment and what is below it";
pub(crate) const FOCUS_HERE: &str = "Focus here";
pub(crate) const SHOW_WHOLE_TREE: &str = "Show whole tree";
pub(crate) const STALE_ITS_PARENT_S_END: &str = "Stale: its parent's end state changed. Play it to the end to record its new hashes (a changed ending gets a !).";
pub(crate) const STALE_A_SEGMENT_ABOVE_IT: &str = "Stale: a segment above it changed and isn't clean yet. Clean that one first (or use Project > Clean all segments).";
pub(crate) const MARKERS: &str = "Markers";
pub(crate) const SEGMENTS: &str = "Segments";
pub(crate) const ONLY_CHILDREN_STAY_IN_THEIR: &str = "Only children stay in their parent's column. Click to indent every level";
pub(crate) const INDENTING_EVERY_LEVEL_CLICK_TO: &str = "Indenting every level. Click to keep only children in their parent's column";
pub(crate) const NO_MATCHING_SEGMENTS: &str = "No matching segments";
pub(crate) const ONE_MATCH: &str = "1 match";
pub(crate) const STATUS_CLEAN: &str = "clean";
pub(crate) const DIRTY_UNRECORDED_NO_END_HASH: &str = "dirty (unrecorded: no end hash yet)";
pub(crate) const DIRTY_STALE_STARTS_WHERE_ITS: &str = "dirty (stale: starts where its parent no longer ends)";
pub(crate) const AGENT: &str = "agent";
pub(crate) const AN_AGENT_S_DRAFT_IN: &str = "An agent's draft in .local/drafts/: an unsaved segment. File > Save moves it into segments/; Delete removes it.";
pub(crate) const RENAME: &str = "Rename";

// ---- The picture and the status bar ----

pub(crate) const BASELINE: &str = "Baseline";
pub(crate) const CHECKING_PARITY: &str = "checking parity";
pub(crate) const CORE: &str = "core";
pub(crate) const RUN_COMPARISON_IS_OFF: &str = "Run comparison is off";
pub(crate) const PAUSED: &str = "paused";
pub(crate) const STOPPED: &str = "stopped";
pub(crate) const OUT_OF_SYNC: &str = "out of sync";
pub(crate) const IN_STEP: &str = "in step";
pub(crate) const SYNCS_AT_THE_NEXT_FRAME: &str = "Syncs at the next frame shown";
pub(crate) const SYNCS_WHEN_YOU_STOP_STEPPING: &str = "Syncs when you stop stepping back";
pub(crate) const WAITING_TO_SYNC: &str = "waiting to sync";
pub(crate) const RECOMPUTING_AFTER_THE_EDIT: &str = "recomputing after the edit…";
pub(crate) const STOP_HYDRATING_FINISHED_SEGMENTS_STAY: &str = "Stop hydrating; finished segments stay stored";
pub(crate) const STOP_CLEANING_HASHES_RECORDED_SO: &str = "Stop cleaning; hashes recorded so far stay (unsaved)";
pub(crate) const RESUME: &str = "Resume";

// ---- Shared widgets ----

pub(crate) const FILTER_MARKER_OR_SEGMENT: &str = "Filter: marker or segment";
pub(crate) const WRITES_AND_OTHER_LINES: &str = "Writes and other lines";
pub(crate) const RESET_MARKER: &str = "Reset";
pub(crate) const COUNT_MATCHING_MARKERS: &str = "matching markers";
pub(crate) const COUNT_MARKERS: &str = "markers";
pub(crate) const ONE_CHILD_SEGMENT: &str = "1 child segment";
pub(crate) const EXPORT_SAVE_RAM: &str = "Export save RAM";
pub(crate) const EXPORT_SAVE_RAM_MENU: &str = "Export save RAM…";
pub(crate) const EXPORT_SAVE_RAM_TIP: &str = "Write the game's battery save, as it is after the frame shown, to a file. Wait until the game has finished saving. Games with no battery save have none.";
pub(crate) const NEW_ROOT_SEGMENT_FROM_THIS_SAVE: &str = "New root segment from this save";
pub(crate) const NEW_ROOT_SEGMENT_FROM_THIS_SAVE_TIP: &str = "A new segment with no parent that powers on with the game's battery save as it is after the frame shown. The save is a file in blobs/";
pub(crate) const NAME_IN_BLOBS: &str = "Name in blobs/";
pub(crate) const NAME_IN_BLOBS_TIP: &str = "What the file in blobs/ is called. dj adds 8 digits of the file's hash after it. Letters, digits, - _ and . only. Changing it renames the file when you apply; segments name the hash, so none of them change.";
pub(crate) const TYPE_TO_NARROW: &str = "Type to narrow the list";
pub(crate) const START_SAVE: &str = "Save";
pub(crate) const SNAPSHOTS: &str = "Snapshots";
pub(crate) const SAVE_FILES: &str = "Save files";
pub(crate) const A_SNAPSHOT_FROM_DISK: &str = "A snapshot from disk…";
pub(crate) const A_SAVE_FILE_FROM_DISK: &str = "A save file from disk…";
pub(crate) const NO_SEGMENT_STARTS_FROM_IT: &str = "No segment starts from it";
