//! Semantic system-variable IDs, independent of reported engine versions.
/// IDs for an explicitly supported runtime layout. Array families use a base ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemVarIds {
    /// Runtime ID for `time`.
    pub time: u16,
    /// Runtime ID for `os_time`.
    pub os_time: u16,
    /// Runtime ID for `loop_counter`.
    pub loop_counter: u16,
    /// Runtime ID for `arg_int`.
    pub arg_int: u16,
    /// Runtime ID for `arg_float`.
    pub arg_float: u16,
    /// Runtime ID for `arg_str`.
    pub arg_str: u16,
    /// Runtime ID for `ret_int`.
    pub ret_int: u16,
    /// Runtime ID for `ret_float`.
    pub ret_float: u16,
    /// Runtime ID for `ret_str`.
    pub ret_str: u16,
    /// Runtime ID for `system_path`.
    pub system_path: u16,
    /// Runtime ID for `project_path`.
    pub project_path: u16,
    /// Runtime ID for `save_path`.
    pub save_path: u16,
    /// Runtime ID for `appdata_path`.
    pub appdata_path: u16,
    /// Runtime ID for `personal_path`.
    pub personal_path: u16,
    /// Runtime ID for `save_path_type`.
    pub save_path_type: u16,
    /// Runtime ID for `display_depth`.
    pub display_depth: u16,
    /// Runtime ID for `display_width`.
    pub display_width: u16,
    /// Runtime ID for `display_height`.
    pub display_height: u16,
    /// Runtime ID for `text`.
    pub text: u16,
    /// Runtime ID for `text_flags`.
    pub text_flags: u16,
    /// Runtime ID for `characters`.
    pub characters: u16,
    /// Runtime ID for `mouse_x`.
    pub mouse_x: u16,
    /// Runtime ID for `mouse_y`.
    pub mouse_y: u16,
    /// Runtime ID for `inp`.
    pub inp: u16,
    /// Runtime ID for `mouse_state`.
    pub mouse_state: u16,
    /// Runtime ID for `key_state`.
    pub key_state: u16,
    /// Runtime ID for `key_any`.
    pub key_any: u16,
    /// Runtime ID for `mouse_count`.
    pub mouse_count: u16,
    /// Runtime ID for `key_count`.
    pub key_count: u16,
    /// Runtime ID for `key_count_any`.
    pub key_count_any: u16,
    /// Runtime ID for `mouse_code`.
    pub mouse_code: u16,
    /// Runtime ID for `key_code`.
    pub key_code: u16,
    /// Runtime ID for `pad_code`.
    pub pad_code: u16,
}
/// Existing semantics verified against SkyChord; shape matches alone do not select this.
pub const SKYCHORD_IDS: SystemVarIds = SystemVarIds {
    time: 0,
    os_time: 1,
    loop_counter: 48,
    arg_int: 53,
    arg_float: 54,
    arg_str: 55,
    ret_int: 60,
    ret_float: 61,
    ret_str: 62,
    system_path: 103,
    project_path: 104,
    save_path: 105,
    appdata_path: 106,
    personal_path: 107,
    save_path_type: 112,
    display_depth: 114,
    display_width: 115,
    display_height: 116,
    text: 122,
    text_flags: 123,
    characters: 127,
    mouse_x: 133,
    mouse_y: 138,
    inp: 144,
    mouse_state: 147,
    key_state: 167,
    key_any: 275,
    mouse_count: 281,
    key_count: 301,
    key_count_any: 409,
    mouse_code: 415,
    key_code: 435,
    pad_code: 543,
};
/// Runtime mapping selected by evidence or an explicit embedding choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SystemVarMap {
    /// No inferred system-variable semantics; ordinary storage remains accessible.
    #[default]
    Unknown,
    /// Existing SkyChord runtime semantics (also the legacy standalone VM API).
    SkyChord,
}
impl SystemVarMap {
    /// Returns the selected semantic IDs, or no mapping for unknown games.
    pub fn ids(self) -> Option<SystemVarIds> {
        match self { Self::Unknown => None, Self::SkyChord => Some(SKYCHORD_IDS) }
    }
}
