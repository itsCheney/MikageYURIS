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
    /// Runtime ID for `save_path_type` (absent before the 470 dictionaries).
    pub save_path_type: Option<u16>,
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
impl SystemVarIds {
    /// Every ID the runtime reads or writes (array families by their base).
    pub fn anchors(&self) -> Vec<u16> {
        let mut v = vec![
            self.time, self.os_time, self.loop_counter, self.arg_int, self.arg_float, self.arg_str,
            self.ret_int, self.ret_float, self.ret_str, self.system_path, self.project_path,
            self.save_path, self.appdata_path, self.personal_path, self.display_depth,
            self.display_width, self.display_height, self.text, self.text_flags, self.characters,
            self.mouse_x, self.mouse_y, self.inp, self.mouse_state, self.key_state, self.key_any,
            self.mouse_count, self.key_count, self.key_count_any, self.mouse_code, self.key_code,
            self.pad_code,
        ];
        v.extend(self.save_path_type);
        v
    }
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
    save_path_type: Some(112),
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
/// IDs of each public SDK dictionary family (YSCom/*.ycd), looked up there
/// by variable name; the family names are the ones of yuris_format's shape
/// profiles. Within every family the input blocks keep the same relative
/// layout (mouse, keys, counters, codes), only their bases move. Facts from
/// shimamura-sakura/yuri commit 183b85e.
pub const DICTIONARY_IDS: &[(&str, SystemVarIds)] = &[
    ("450,451", SystemVarIds { time: 0, os_time: 1, loop_counter: 32, arg_int: 34, arg_float: 35, arg_str: 36, ret_int: 38, ret_float: 39, ret_str: 40, system_path: 55, project_path: 56, save_path: 57, appdata_path: 58, personal_path: 59, save_path_type: None, display_depth: 62, display_width: 63, display_height: 64, text: 70, text_flags: 71, characters: 75, mouse_x: 81, mouse_y: 86, inp: 92, mouse_state: 95, key_state: 115, key_any: 223, mouse_count: 229, key_count: 249, key_count_any: 357, mouse_code: 363, key_code: 383, pad_code: 491 }),
    ("452,453,454,455,456,457,458,459,460,461,462,463,464,465,466", SystemVarIds { time: 0, os_time: 1, loop_counter: 48, arg_int: 50, arg_float: 51, arg_str: 52, ret_int: 54, ret_float: 55, ret_str: 56, system_path: 71, project_path: 72, save_path: 73, appdata_path: 74, personal_path: 75, save_path_type: None, display_depth: 79, display_width: 80, display_height: 81, text: 87, text_flags: 88, characters: 92, mouse_x: 98, mouse_y: 103, inp: 109, mouse_state: 112, key_state: 132, key_any: 240, mouse_count: 246, key_count: 266, key_count_any: 374, mouse_code: 380, key_code: 400, pad_code: 508 }),
    ("470,471", SystemVarIds { time: 0, os_time: 1, loop_counter: 48, arg_int: 50, arg_float: 51, arg_str: 52, ret_int: 54, ret_float: 55, ret_str: 56, system_path: 71, project_path: 72, save_path: 73, appdata_path: 74, personal_path: 75, save_path_type: Some(80), display_depth: 82, display_width: 83, display_height: 84, text: 90, text_flags: 91, characters: 95, mouse_x: 101, mouse_y: 106, inp: 112, mouse_state: 115, key_state: 135, key_any: 243, mouse_count: 249, key_count: 269, key_count_any: 377, mouse_code: 383, key_code: 403, pad_code: 511 }),
    ("472,473,477,479", SystemVarIds { time: 0, os_time: 1, loop_counter: 48, arg_int: 50, arg_float: 51, arg_str: 52, ret_int: 54, ret_float: 55, ret_str: 56, system_path: 81, project_path: 82, save_path: 83, appdata_path: 84, personal_path: 85, save_path_type: Some(90), display_depth: 92, display_width: 93, display_height: 94, text: 100, text_flags: 101, characters: 105, mouse_x: 111, mouse_y: 116, inp: 122, mouse_state: 125, key_state: 145, key_any: 253, mouse_count: 259, key_count: 279, key_count_any: 387, mouse_code: 393, key_code: 413, pad_code: 521 }),
    ("480", SystemVarIds { time: 0, os_time: 1, loop_counter: 48, arg_int: 50, arg_float: 51, arg_str: 52, ret_int: 54, ret_float: 55, ret_str: 56, system_path: 93, project_path: 94, save_path: 95, appdata_path: 96, personal_path: 97, save_path_type: Some(102), display_depth: 104, display_width: 105, display_height: 106, text: 112, text_flags: 113, characters: 117, mouse_x: 123, mouse_y: 128, inp: 134, mouse_state: 137, key_state: 157, key_any: 265, mouse_count: 271, key_count: 291, key_count_any: 399, mouse_code: 405, key_code: 425, pad_code: 533 }),
    ("484,487,488,493", SystemVarIds { time: 0, os_time: 1, loop_counter: 48, arg_int: 53, arg_float: 54, arg_str: 55, ret_int: 57, ret_float: 58, ret_str: 59, system_path: 96, project_path: 97, save_path: 98, appdata_path: 99, personal_path: 100, save_path_type: Some(105), display_depth: 107, display_width: 108, display_height: 109, text: 115, text_flags: 116, characters: 120, mouse_x: 126, mouse_y: 131, inp: 137, mouse_state: 140, key_state: 160, key_any: 268, mouse_count: 274, key_count: 294, key_count_any: 402, mouse_code: 408, key_code: 428, pad_code: 536 }),
    ("494", SKYCHORD_IDS),
];
/// Runtime mapping selected by evidence or an explicit embedding choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SystemVarMap {
    /// No inferred system-variable semantics; ordinary storage remains accessible.
    #[default]
    Unknown,
    /// Existing SkyChord runtime semantics (also the legacy standalone VM API).
    SkyChord,
    /// A dictionary family's IDs, chosen from the YSVR shape alone (not
    /// checked by playing a game of it). Holds a [`DICTIONARY_IDS`] name.
    Dictionary(&'static str),
}
impl SystemVarMap {
    /// Returns the selected semantic IDs, or no mapping for unknown games.
    pub fn ids(self) -> Option<SystemVarIds> {
        match self {
            Self::Unknown => None,
            Self::SkyChord => Some(SKYCHORD_IDS),
            Self::Dictionary(name) => DICTIONARY_IDS.iter().find(|(n, _)| *n == name).map(|(_, ids)| *ids),
        }
    }
}
