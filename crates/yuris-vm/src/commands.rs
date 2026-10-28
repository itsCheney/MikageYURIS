//! Command map: turns the command indices of a game's `ysc.ybn` (YSCM) into
//! the VM's canonical indices.
//!
//! Scripts index commands in **the game's own YSCM order**, which changes
//! with the engine version and custom builds (the v555 sample has
//! `00 = ALIAS`, `0d = END`; Erewhon v481 has `00 = CDDA`, an extra
//! `EMOTEINFO`, and `END` at `0e`). The VM's handlers are written against
//! the v555 order ([`CANONICAL`], the [`crate::cmd`] constants), so scripts
//! are remapped **by name** when loaded. A command missing from the
//! canonical table becomes `0x80 | game index`: it takes the unimplemented
//! path and can still be named.
//!
//! The low byte of a window tag is the **parameter slot**, again in the
//! game's YSCM parameter order, which also changes between versions
//! (VARINFO `STR`/`NO` are 19/20 in v555 and 17/18 in v481). Slots are
//! remapped by parameter name as well; a parameter missing from the
//! canonical table becomes `0x80 | game slot`.

use yuris_format::yscm::YscmTable;

/// Canonical commands, in the order of the v555 sample's `ysc.ybn`.
pub const CANONICAL: [&str; 121] = [
    "ALIAS", "CG", "CGACT", "CGEND", "CGINFO", "CLIPACT", "CLIPINFO", "CSV", "CSVINFO",
    "DEBUGLIST", "DIALOG", "ELSE", "EMOTE", "END", "ERROR", "ERRORINFO", "F_FLT", "F_INT",
    "F_STR", "F_VAR", "FILEACT", "FILEINFO", "FLASH", "FLASHEND", "FLASHINFO", "FLT", "FONT",
    "FONTINFO", "FPS", "G_FLT", "G_FLT2", "G_FLT3", "G_FLT4", "G_INT", "G_INT2", "G_INT3",
    "G_INT4", "G_STR", "G_STR2", "G_STR3", "G_STR4", "G_VAR", "GO", "GOSUB", "IF", "IFBLEND",
    "IFBREAK", "IFCONTINUE", "IFEND", "INPUT", "INT", "LABEL", "LABELINFO", "LET", "LOAD",
    "LOOP", "LOOPBREAK", "LOOPCONTINUE", "LOOPEND", "MACRO", "MATH", "MATH3D", "MENU",
    "MENUACT", "MENUEND", "MENUINFO", "MIDI", "MIDIEND", "MIDIINFO", "MOUSE", "MOVIE",
    "MOVIEEND", "MOVIEINFO", "MOVIE2", "MOVIE2END", "MOVIE2INFO", "PLUG", "PREP", "REGINFO",
    "RETURN", "RETURNBREAK", "RETURNCODE", "S_FLT", "S_INT", "S_STR", "S_VAR", "SAVE",
    "SAVEEND", "SAVEINFO", "SOUND", "SOUNDEND", "SOUNDINFO", "STR", "SYSTEM", "SYSTEMINFO",
    "TASK", "TASKEND", "TASKINFO", "TEXT", "TEXTINFO", "TRAY", "VAR", "VARACT", "VARINFO",
    "WAIT", "WINDOW", "WINDOWEND", "WINDOWINFO", "WORD", "YMV", "YMVEND", "YMVINFO", "_",
    "_IFDEF", "_IFNDEF", "_IFEND", "PRODUCT", "SYSTEMMODE", "COMPILEMODE", "RELEASEMODE",
    "PROJECTFOLDER",
];

/// Canonical parameters, `CANONICAL_PARAMS[cmd][slot]`, in the v555 sample's
/// YSCM order.
pub const CANONICAL_PARAMS: [&[&str]; 121] = [
    // ALIAS
    &[],
    // CG
    &[
        "ID", "IDNO", "E", "A", "X", "Y", "Z", "SD", "MD", "SX", "SY", "SLX", "SLY", "SCX",
        "SCY", "RZ", "RLX", "RLY", "RCX", "RCY", "SIP", "RIP", "TSX", "TSY", "MIPMAP", "TEX",
        "DXBUF", "DXDRAW", "FX", "FY", "FBX", "FBY", "FRX", "FRY", "FRZ", "FCX", "FCY", "FEX",
        "FEY", "FEZ", "FSD", "FMD", "FQU", "FID", "F", "REMALLOC", "FILE", "RID", "MID", "MFILE",
        "T", "TID", "TIDNO", "TA", "MODE", "CASH", "LINT", "LINT2",
    ],
    // CGACT
    &[
        "ID", "ID2", "ID3", "ID4", "IDNO", "IDNO2", "IDNO3", "IDNO4", "Z2", "SET", "SET2",
        "SETSTR", "X", "Y", "X2", "Y2", "SX", "SY", "FX", "FY", "FX2", "FY2", "FSX", "FSY",
        "SIZE", "SIZE2", "SIZE3", "SIZE4", "SIZE5", "ROTATION", "ROTATION2", "SAIDO", "MEIDO",
        "SEPIA", "MERGE", "MERGE2", "TRANSITION", "TRANSITION2", "XOR", "AND", "TRIM", "RESIZE",
        "AKARUSA", "MONO", "VERTICAL", "HORIZONTAL", "MOSAIC", "BLUR", "BLUR2", "BLUR3", "BLUR4",
        "BLUR5", "BLUR6", "DIFFUSION", "RASTER", "RASTER2", "RASTER3", "REVERSE", "RECTPAINT",
        "RECTPAINT2", "RECTPAINT3", "GRADATION", "WHIRLPOOL", "BNO", "TEXT", "COPY", "COPY2",
        "SWAP", "SWAP2", "TEXCOPY", "TEXSSCOPY",
    ],
    // CGEND
    &["ID", "IDNO"],
    // CGINFO
    &[
        "ID", "IDNO", "EXIST", "E", "A", "X", "Y", "Z", "MFILE", "T", "TID", "TA", "MODE", "SX",
        "SY", "FBX", "FBY", "FEX", "FEY", "FEZ", "TEX", "ONMOUSE", "ONMOUSE2", "ONMOUSE3",
        "COLOR", "TRIM", "LINT", "LINT2", "SEARCH", "NUM", "NAME", "LSET", "NO", "LET", "SET",
        "SET2",
    ],
    // CLIPACT
    &["TEXT", "SET"],
    // CLIPINFO
    &["TEXT", "LET"],
    // CSV
    &["FILE", "LET"],
    // CSVINFO
    &["FILE", "SIZE", "LET"],
    // DEBUGLIST
    &["ADD", "DEL", "STR"],
    // DIALOG
    &[
        "CAPTION", "STR", "STR2", "STR3", "STR4", "STR5", "STR6", "STR7", "STR8", "DEFSTR",
        "LETSTR", "LET", "OK", "OKCANCEL", "YESNO", "YESNOCANCEL", "INPUT", "INPUTLIMIT",
        "DEFBUTTON", "ICON", "FILEOPEN", "FILEEXT", "FOLDER", "CHECKBOX", "SLIDER", "A", "A2",
        "A3", "A4", "A5", "A6", "A7", "A8", "E", "E2", "E3", "E4", "E5", "E6", "E7", "E8",
        "CHECK", "CHECK2", "CHECK3", "CHECK4", "CHECK5", "CHECK6", "CHECK7", "CHECK8", "NOWAIT",
    ],
    // ELSE
    &[],
    // EMOTE
    &[
        "ID", "IDNO", "HAIR", "BUST", "PARTS", "WIND", "WIND_START", "WIND_GOAL", "WIND_SPEED",
        "WIND_POWMIN", "WIND_POWMAX", "SKIP", "LIST", "PLAY", "TIME", "VAR", "VAR2", "VARNUM",
    ],
    // END
    &["RESTART"],
    // ERROR
    &[
        "NO", "SET", "COM_ALIAS", "COM_CG", "COM_CGACT", "COM_CGEND", "COM_CGINFO",
        "COM_CLIPACT", "COM_CLIPINFO", "COM_CSV", "COM_CSVINFO", "COM_DEBUGLIST", "COM_DIALOG",
        "COM_ELSE", "COM_END", "COM_ERROR", "COM_ERRORINFO", "COM_FILEACT", "COM_FILEINFO",
        "COM_FLASH", "COM_FLASHEND", "COM_FLASHINFO", "COM_FLT", "COM_FONT", "COM_FONTINFO",
        "COM_FPS", "COM_G_FLT", "COM_G_FLT2", "COM_G_FLT3", "COM_G_FLT4", "COM_G_INT",
        "COM_G_INT2", "COM_G_INT3", "COM_G_INT4", "COM_G_STR", "COM_G_STR2", "COM_G_STR3",
        "COM_G_STR4", "COM_G_VAR", "COM_GO", "COM_GOSUB", "COM_IF", "COM_IFBLEND", "COM_IFBREAK",
        "COM_IFCONTINUE", "COM_IFEND", "COM_INT", "COM_INPUT", "COM_LABEL", "COM_LABELINFO",
        "COM_LET", "COM_LOAD", "COM_LOOP", "COM_LOOPBREAK", "COM_LOOPCONTINUE", "COM_LOOPEND",
        "COM_MACRO", "COM_MATH", "COM_MENU", "COM_MENUACT", "COM_MENUEND", "COM_MENUINFO",
        "COM_MIDI", "COM_MOUSE", "COM_MOVIE", "COM_MOVIEEND", "COM_MOVIEINFO", "COM_PLUG",
        "COM_PREP", "COM_REGINFO", "COM_RETURN", "COM_RETURNBREAK", "COM_RETURNCODE",
        "COM_S_FLT", "COM_S_INT", "COM_S_STR", "COM_S_VAR", "COM_SAVE", "COM_SAVEEND",
        "COM_SAVEINFO", "COM_SOUND", "COM_SOUNDEND", "COM_SOUNDINFO", "COM_STR", "COM_SYSTEM",
        "COM_SYSTEMINFO", "COM_TASK", "COM_TASKEND", "COM_TASKINFO", "COM_TEXT", "COM_TEXTINFO",
        "COM_TRAY", "COM_VAR", "COM_VARACT", "COM_VARINFO", "COM_WAIT", "COM_WINDOW",
        "COM_WINDOWEND", "COM_WINDOWINFO", "COM_WORD", "COM_YMV", "COM_YMVEND", "COM_YMVINFO",
        "COM__", "COM__IFDEF", "COM__IFNDEF", "COM__IFEND", "COM_PRODUCT",
    ],
    // ERRORINFO
    &["NO", "LET"],
    // F_FLT
    &[],
    // F_INT
    &[],
    // F_STR
    &[],
    // F_VAR
    &[],
    // FILEACT
    &[
        "FILE", "FILE2", "FOLDER", "CREATE", "COPY", "DELETE", "EXEC", "EXEC2", "PARAM", "SET",
        "ID", "IDNO", "BMP", "PNG", "JPG", "WEBP", "CGCASH", "SOUNDCASH",
    ],
    // FILEINFO
    &["FOLDER", "FILE", "CGFILE", "SOUNDFILE", "EXIST", "SIZE", "RELATION", "LET"],
    // FLASH
    &["FILE", "PLAY", "X", "Y", "SX", "SY", "ENDFRAME"],
    // FLASHEND
    &[],
    // FLASHINFO
    &["PLAY", "VAR", "X", "Y", "SX", "SY", "FRAME", "ENDFRAME", "TOTALFRAME", "LET"],
    // FLT
    &["SET"],
    // FONT
    &[
        "BNO", "NO", "NAME", "ANTIALIASING", "COLOR", "COLOR2", "GRADATION", "ROTATION",
        "DRAWMODE", "SX", "SY", "BOLD",
    ],
    // FONTINFO
    &[
        "NO", "NAME", "ANTIALIASING", "COLOR", "COLOR2", "GRADATION", "ROTATION", "DRAWMODE",
        "SX", "SY", "BOLD", "NUM", "PITCH", "LANG", "X", "Y", "SET", "SETSTR", "LET",
    ],
    // FPS
    &["SET", "SKIP"],
    // G_FLT
    &[],
    // G_FLT2
    &[],
    // G_FLT3
    &[],
    // G_FLT4
    &[],
    // G_INT
    &[],
    // G_INT2
    &[],
    // G_INT3
    &[],
    // G_INT4
    &[],
    // G_STR
    &[],
    // G_STR2
    &[],
    // G_STR3
    &[],
    // G_STR4
    &[],
    // G_VAR
    &[],
    // GO
    &["#"],
    // GOSUB
    &[
        "#", "PINT", "PINT2", "PINT3", "PINT4", "PINT5", "PINT6", "PINT7", "PINT8", "PINT9",
        "PINT10", "PINT11", "PINT12", "PINT13", "PINT14", "PINT15", "PINT16", "PFLT", "PFLT2",
        "PFLT3", "PFLT4", "PFLT5", "PFLT6", "PFLT7", "PFLT8", "PFLT9", "PFLT10", "PFLT11",
        "PFLT12", "PFLT13", "PFLT14", "PFLT15", "PFLT16", "PSTR", "PSTR2", "PSTR3", "PSTR4",
        "PSTR5", "PSTR6", "PSTR7", "PSTR8", "PSTR9", "PSTR10", "PSTR11", "PSTR12", "PSTR13",
        "PSTR14", "PSTR15", "PSTR16",
    ],
    // IF
    &[],
    // IFBLEND
    &[],
    // IFBREAK
    &["LV"],
    // IFCONTINUE
    &["LV"],
    // IFEND
    &[],
    // INPUT
    &["KEY", "PAD", "MOUSE"],
    // INT
    &["SET"],
    // LABEL
    &["#"],
    // LABELINFO
    &["#", "LET", "EXIST"],
    // LET
    &[],
    // LOAD
    &[
        "FILE", "MEM", "DNO", "LET", "ID", "IDNO", "G_INT", "G_INT2", "G_INT3", "G_INT4",
        "G_FLT", "G_FLT2", "G_FLT3", "G_FLT4", "G_STR", "G_STR2", "G_STR3", "G_STR4",
    ],
    // LOOP
    &["SET"],
    // LOOPBREAK
    &["LV"],
    // LOOPCONTINUE
    &["LV"],
    // LOOPEND
    &[],
    // MACRO
    &[
        "NAME", "NAME2", "NAME3", "NAME4", "NAME5", "NAME6", "NAME7", "NAME8", "NAME9", "NAME10",
        "NAME11", "NAME12", "NAME13", "NAME14", "NAME15", "NAME16", "NUM", "STR", "DEF", "DEF2",
        "DEF3", "DEF4", "DEF5", "DEF6", "DEF7", "DEF8", "DEF9", "DEF10", "DEF11", "DEF12",
        "DEF13", "DEF14", "DEF15", "DEF16", "PRE", "PRE2", "PRE3", "PRE4", "PRE5", "PRE6",
        "PRE7", "PRE8", "PRE9", "PRE10", "PRE11", "PRE12", "PRE13", "PRE14", "PRE15", "PRE16",
        "SUF", "SUF2", "SUF3", "SUF4", "SUF5", "SUF6", "SUF7", "SUF8", "SUF9", "SUF10", "SUF11",
        "SUF12", "SUF13", "SUF14", "SUF15", "SUF16", "DEFB", "DEFB2", "DEFB3", "DEFB4", "DEFB5",
        "DEFB6", "DEFB7", "DEFB8", "DEFB9", "DEFB10", "DEFB11", "DEFB12", "DEFB13", "DEFB14",
        "DEFB15", "DEFB16", "PREB", "PREB2", "PREB3", "PREB4", "PREB5", "PREB6", "PREB7",
        "PREB8", "PREB9", "PREB10", "PREB11", "PREB12", "PREB13", "PREB14", "PREB15", "PREB16",
        "SUFB", "SUFB2", "SUFB3", "SUFB4", "SUFB5", "SUFB6", "SUFB7", "SUFB8", "SUFB9", "SUFB10",
        "SUFB11", "SUFB12", "SUFB13", "SUFB14", "SUFB15", "SUFB16", "PREFIX",
    ],
    // MATH
    &[
        "LET", "SET", "SET2", "SETSTR", "SIN", "ASIN", "SINH", "COS", "ACOS", "COSH", "TAN",
        "ATAN", "ATAN2", "TANH", "SQRT", "ABS", "EXP", "LOG", "LOG10", "POW", "RND", "HASH",
        "CALC",
    ],
    // MATH3D
    &["INT", "FLT"],
    // MENU
    &["ID", "NAME", "Z", "E", "A", "CHECK", "CODE"],
    // MENUACT
    &["ID", "CONTEXT"],
    // MENUEND
    &["ID"],
    // MENUINFO
    &["LET", "ID", "NAME", "EXIST", "Z", "E", "A", "CHECK", "CODE"],
    // MIDI
    &["FILE", "PLAY"],
    // MIDIEND
    &[],
    // MIDIINFO
    &[],
    // MOUSE
    &["MODE", "TIME", "MOVE", "CLICK", "WHEEL", "X", "Y", "CURSOR", "HX", "HY", "ID"],
    // MOVIE
    &["FILE", "PLAY", "X", "Y", "SX", "SY", "VOLUME"],
    // MOVIEEND
    &[],
    // MOVIEINFO
    &["PLAY", "X", "Y", "SX", "SY", "VOLUME", "CODEC", "LET"],
    // MOVIE2
    &["ID", "IDNO", "FILE", "PLAY", "SX", "SY", "VOLUME"],
    // MOVIE2END
    &[],
    // MOVIE2INFO
    &["PLAY", "SX", "SY", "VOLUME", "CODEC", "LET"],
    // PLUG
    &[
        "NAME", "FUNC", "SET", "SETFLT", "SETSTR", "", "", "", "LET", "LETFLT", "LETSTR", "", "",
        "",
    ],
    // PREP
    &["SERIAL_GNO", "SERIAL_LNO", "TEXTVAL", "IFNEST", "LOOPNEST", "BRACENEST"],
    // REGINFO
    &["KEY", "NAME", "LET"],
    // RETURN
    &[
        "RINT", "RINT2", "RINT3", "RINT4", "RINT5", "RINT6", "RINT7", "RINT8", "RINT9", "RINT10",
        "RINT11", "RINT12", "RINT13", "RINT14", "RINT15", "RINT16", "RFLT", "RFLT2", "RFLT3",
        "RFLT4", "RFLT5", "RFLT6", "RFLT7", "RFLT8", "RFLT9", "RFLT10", "RFLT11", "RFLT12",
        "RFLT13", "RFLT14", "RFLT15", "RFLT16", "RSTR", "RSTR2", "RSTR3", "RSTR4", "RSTR5",
        "RSTR6", "RSTR7", "RSTR8", "RSTR9", "RSTR10", "RSTR11", "RSTR12", "RSTR13", "RSTR14",
        "RSTR15", "RSTR16",
    ],
    // RETURNBREAK
    &[],
    // RETURNCODE
    &[],
    // S_FLT
    &[],
    // S_INT
    &[],
    // S_STR
    &[],
    // S_VAR
    &[],
    // SAVE
    &[
        "FILE", "MEM", "DNO", "SET", "ID", "IDNO", "G_INT", "G_INT2", "G_INT3", "G_INT4",
        "G_FLT", "G_FLT2", "G_FLT3", "G_FLT4", "G_STR", "G_STR2", "G_STR3", "G_STR4",
    ],
    // SAVEEND
    &[],
    // SAVEINFO
    &[],
    // SOUND
    &[
        "ID", "E", "FILE", "PLAY", "PAUSE", "FADE", "START", "RESTART", "END", "LOOP", "VOLUME",
        "PAN", "PITCH", "SPEED",
    ],
    // SOUNDEND
    &["ID"],
    // SOUNDINFO
    &[
        "ID", "EXIST", "E", "PLAY", "PAUSE", "VOLUME", "PAN", "PITCH", "FILE", "FILETYPE",
        "START", "RESTART", "END", "LOOP", "LOOPCOUNT", "TIME", "TOTALTIME", "CHANNEL", "BIT",
        "SAMPLING", "DATALEFT", "DATARIGHT", "CODEC", "LET",
    ],
    // STR
    &["SET"],
    // SYSTEM
    &[
        "SCREENSAVER", "BACKGROUNDEXEC", "SX", "SY", "TIMECOUNT", "TIMECOUNT2", "TIMECOUNT3",
        "TIMECOUNT4",
    ],
    // SYSTEMINFO
    &["SCRIPTFILE", "SCRIPTNUM", "TEXTNUM", "EMOTE", "NO", "LET"],
    // TASK
    &[
        "ID", "E", "A", "Z", "TINT", "TINT2", "TINT3", "TINT4", "TINT5", "TINT6", "TINT7",
        "TINT8", "TINT9", "TINT10", "TFLT", "TFLT2", "TFLT3", "TFLT4", "TFLT5", "TFLT6", "TFLT7",
        "TFLT8", "TFLT9", "TFLT10", "TSTR", "TSTR2", "TSTR3", "TSTR4", "TSTR5", "TSTR6", "TSTR7",
        "TSTR8", "TSTR9", "TSTR10", "#", "SCRIPTPOS", "TEXTPOS", "RESTART",
    ],
    // TASKEND
    &["ID"],
    // TASKINFO
    &[
        "ID", "LET", "EXIST", "E", "A", "Z", "FILE", "FILENO", "LINE", "FILE0", "FILENO0",
        "LINE0", "TEXTLINE", "NEXTVOICE", "GOLABEL", "GOSUBLABEL", "IFNEST", "GOSUBNEST",
        "LOOPNEST", "SCRIPTPOS", "TEXTPOS",
    ],
    // TEXT (v555 has FILE/LET/CLEAR; the rest are v481's, as in the manual)
    &[
        "FILE", "LET", "CLEAR", "ID", "OUTPUTMODE", "CLICKBUTTON", "CLICKBUTTON2", "CLICKBUTTON3",
        "CLICKBUTTON4", "SKIPBUTTON", "SKIPBUTTON2", "SKIPBUTTON3", "SKIPBUTTON4", "WAIT",
        "RETURN", "PAGE", "CLICKRETURN", "CLICKPAGE", "CLICK", "AUTORETURN", "AUTOPAGE",
        "AUTOCLICK",
    ],
    // TEXTINFO (v481)
    &[
        "ID", "OUTPUTMODE", "CX", "CY", "CLICK", "AUTORETURN", "AUTOPAGE", "AUTOCLICK", "LET",
        "SET",
    ],
    // TRAY
    &["E", "NAME", "ID", "MENUID"],
    // VAR
    &[""],
    // VARACT
    &[
        "SET", "LET", "CUT", "COPY", "POS", "LENGTH", "TYPE", "UPPER", "UPPER2", "LOWER",
        "LOWER2", "HANTOZEN", "ZENTOHAN", "DIMSIZE", "PUSH", "POP", "INIT", "G_INT", "G_INT2",
        "G_INT3", "G_INT4", "G_FLT", "G_FLT2", "G_FLT3", "G_FLT4", "G_STR", "G_STR2", "G_STR3",
        "G_STR4",
    ],
    // VARINFO
    &[
        "SET", "LET", "TYPE", "STRTYPE", "DIMNUM", "DIMSIZE", "DIMSIZE2", "DIMSIZE3", "DIMSIZE4",
        "DIMSIZE5", "DIMSIZE6", "DIMSIZE7", "DIMSIZE8", "LENGTH", "SEARCH", "STRFIRST",
        "SJISCODE", "INT", "FLT", "STR", "NO", "NO2",
    ],
    // WAIT
    &["FRAME", "TIME"],
    // WINDOW
    &[
        "NO", "CAPTION", "A", "E", "X", "Y", "Z", "SX", "SY", "WSX", "WSY", "ACTIVE",
        "FULLSCREENBUTTON", "FULLSCREEN", "MINIMIZE", "MAXIMIZE", "MAXMODE", "MINMODE",
        "CLOSEMODE", "FLASH", "FLASHTIME", "CGID", "MENUID", "REGIONID", "DRAGID", "RESIZE",
    ],
    // WINDOWEND
    &["NO"],
    // WINDOWINFO
    &[
        "NO", "LET", "EXIST", "A", "E", "ACTIVE", "CAPTION", "X", "Y", "Z", "SX", "SY", "WSX",
        "WSY", "DISPLAYNO", "FULLSCREEN", "FULLSCREEN2", "MINIMIZE", "MAXMODE", "MINMODE",
        "CLOSEMODE",
    ],
    // WORD
    &[""],
    // YMV
    &["ID", "IDNO", "FILE", "FRAME"],
    // YMVEND
    &["ID", "IDNO"],
    // YMVINFO
    &["ID", "IDNO", "LET", "NUM", "SX", "SY", "FPS"],
    // _
    &[""],
    // _IFDEF
    &["STR"],
    // _IFNDEF
    &["STR"],
    // _IFEND
    &[],
    // PRODUCT
    &[
        "COM", "LET", "PINT", "PINT2", "PINT3", "PINT4", "PINT5", "PINT6", "PINT7", "PINT8",
        "PINT9", "PINT10", "PFLT", "PFLT2", "PFLT3", "PFLT4", "PFLT5", "PFLT6", "PFLT7", "PFLT8",
        "PFLT9", "PFLT10", "PSTR", "PSTR2", "PSTR3", "PSTR4", "PSTR5", "PSTR6", "PSTR7", "PSTR8",
        "PSTR9", "PSTR10",
    ],
    // SYSTEMMODE
    &[
        "SX", "SY", "E", "BMP", "PNG", "JPG", "GIF", "AVI", "PSB", "WEBP", "WAV", "OGG",
        "CAPTION", "THREAD", "SCRIPTFILEEXT", "DEBUGMODE", "SOUND", "COMPILE", "WINDOWRESIZE",
        "WINDOWFRAME", "FILEPRIORITYDEVELOP", "FILEPRIORITYDEBUG", "FILEPRIORITYRELEASE",
        "DEFSTR", "DEFSTR2", "DEFSTR3", "DEFSTR4", "DEFSTR5", "DEFSTR6", "DEFSTR7", "DEFSTR8",
        "DEFSTR9", "DEFSTR10",
    ],
    // COMPILEMODE
    &[""],
    // RELEASEMODE
    &[""],
    // PROJECTFOLDER
    &["PROJECTFOLDER"],
];

/// Marks a command (or slot) missing from the canonical table.
pub const FOREIGN: u8 = 0x80;

/// Game command index -> canonical index, plus parameter slot maps.
#[derive(Debug, Clone)]
pub struct CommandMap {
    to_canon: [u8; 256],
    /// The compact FILE/LET/CLEAR text interface delegates output to scripts.
    pub(crate) script_text_only: bool,
    game_names: Vec<String>,
    /// Per game command: game slot -> canonical slot; None = unchanged.
    slot_maps: Vec<Option<Vec<u8>>>,
}

impl Default for CommandMap {
    fn default() -> Self {
        Self::identity()
    }
}

impl CommandMap {
    /// The identity map (games in v555 order, synthetic test scripts).
    pub fn identity() -> Self {
        let mut to_canon = [0u8; 256];
        for (i, c) in to_canon.iter_mut().enumerate() {
            *c = i as u8;
        }
        Self {
            to_canon,
            script_text_only: false,
            game_names: CANONICAL.iter().map(|s| s.to_string()).collect(),
            slot_maps: Vec::new(),
        }
    }

    /// Builds the map from a game's YSCM table, by name.
    pub fn from_yscm(t: &YscmTable) -> Self {
        let mut to_canon = [0u8; 256];
        let mut game_names = Vec::with_capacity(t.commands().len());
        for (i, c) in to_canon.iter_mut().enumerate() {
            *c = FOREIGN | (i as u8 & 0x7f);
        }
        let mut slot_maps = Vec::with_capacity(t.commands().len());
        for (i, c) in t.commands().iter().enumerate().take(128) {
            let mut slots = None;
            if let Some(k) = CANONICAL.iter().position(|n| n.eq_ignore_ascii_case(&c.name)) {
                to_canon[i] = k as u8;
                let canon = CANONICAL_PARAMS[k];
                // Repeated names (such as PLUG's empty ones) pair up in order;
                // empty names never match.
                let map: Vec<u8> = c
                    .params
                    .iter()
                    .enumerate()
                    .map(|(gs, p)| {
                        let nth = c.params[..gs]
                            .iter()
                            .filter(|q| q.name.eq_ignore_ascii_case(&p.name))
                            .count();
                        let cs = canon
                            .iter()
                            .enumerate()
                            .filter(|(_, n)| n.eq_ignore_ascii_case(&p.name))
                            .nth(nth)
                            .map(|(i, _)| i);
                        match cs {
                            Some(cs) if !p.name.is_empty() => cs as u8,
                            _ => 0x80 | (gs as u8 & 0x7f),
                        }
                    })
                    .collect();
                if map.iter().enumerate().any(|(gs, &cs)| gs as u8 != cs) {
                    slots = Some(map);
                }
            }
            game_names.push(c.name.clone());
            slot_maps.push(slots);
        }
        let script_text_only = t.commands().iter().any(|c| {
            c.name.eq_ignore_ascii_case("TEXT") && c.params.len() == 3
                && c.params.iter().zip(["FILE", "LET", "CLEAR"])
                    .all(|(p, name)| p.name.eq_ignore_ascii_case(name))
        });
        Self { to_canon, game_names, slot_maps, script_text_only }
    }

    /// Maps one game command index.
    pub fn canon(&self, game_index: u8) -> u8 {
        self.to_canon[game_index as usize]
    }

    /// The name of a canonical index (or of `0x80 | game index`).
    pub fn name(&self, code: u8) -> String {
        if code & FOREIGN != 0 {
            let i = (code & 0x7f) as usize;
            return self
                .game_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("#{i:02x}"));
        }
        CANONICAL
            .get(code as usize)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("#{code:02x}"))
    }

    /// Commands in the game's YSCM that the canonical table lacks.
    pub fn foreign_names(&self) -> Vec<&str> {
        self.game_names
            .iter()
            .enumerate()
            .filter(|(i, _)| self.to_canon[*i] & FOREIGN != 0)
            .map(|(_, n)| n.as_str())
            .collect()
    }

    /// Commands whose parameter slots move: (name, [(game slot, canonical
    /// slot)]). For diagnostics.
    pub fn slot_changes(&self) -> Vec<(String, Vec<(u8, u8)>)> {
        self.slot_maps
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let m = m.as_ref()?;
                let pairs = m
                    .iter()
                    .enumerate()
                    .filter(|(gs, &cs)| *gs as u8 != cs)
                    .map(|(gs, &cs)| (gs as u8, cs))
                    .collect();
                Some((self.game_names[i].clone(), pairs))
            })
            .collect()
    }

    /// Remaps a script context in place: command indices and window
    /// parameter slots. Remap each context only once.
    pub fn remap(&self, ctx: &mut crate::host::ScriptCtx) {
        let crate::host::ScriptCtx { groups, first_slots, script, .. } = ctx;
        let slots = script.slots_mut();
        for (g, &first) in groups.iter_mut().zip(first_slots.iter()) {
            let game = g.command_type as usize;
            if let Some(Some(map)) = self.slot_maps.get(game) {
                for w in &mut slots[first..first + g.window_count as usize] {
                    let s = (w.tag & 0xff) as usize;
                    if let Some(&cs) = map.get(s) {
                        w.tag = (w.tag & !0xff) | u32::from(cs);
                    }
                }
            }
            g.command_type = self.canon(g.command_type);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_text_interface_uses_script_output() {
        assert!(CommandMap::from_yscm(&yscm(&[("TEXT", &["FILE", "LET", "CLEAR"])])).script_text_only);
        assert!(!CommandMap::identity().script_text_only);
        assert!(!CommandMap::from_yscm(&yscm(&[("TEXT", &["FILE", "LET", "CLEAR", "ID", "OUTPUTMODE"])])).script_text_only);
        assert!(!CommandMap::from_yscm(&yscm(&[("TEXT", &["FILE"])])).script_text_only);
    }

    fn yscm(cmds: &[(&str, &[&str])]) -> YscmTable {
        let mut v = b"YSCM".to_vec();
        v.extend_from_slice(&481u32.to_le_bytes());
        v.extend_from_slice(&(cmds.len() as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        for (name, params) in cmds {
            v.extend_from_slice(name.as_bytes());
            v.push(0);
            v.push(params.len() as u8);
            for p in *params {
                v.extend_from_slice(p.as_bytes());
                v.push(0);
                v.extend_from_slice(&0u16.to_le_bytes());
            }
        }
        YscmTable::from_bytes(&v).unwrap()
    }

    #[test]
    fn maps_commands_and_slots_by_name() {
        // v481 style: CDDA first, VARINFO without STRFIRST/SJISCODE
        let t = yscm(&[
            ("CDDA", &[]),
            ("END", &[]),
            (
                "VARINFO",
                &[
                    "SET", "LET", "TYPE", "STRTYPE", "DIMNUM", "DIMSIZE", "DIMSIZE2", "DIMSIZE3",
                    "DIMSIZE4", "DIMSIZE5", "DIMSIZE6", "DIMSIZE7", "DIMSIZE8", "LENGTH", "SEARCH",
                    "INT", "FLT", "STR", "NO",
                ],
            ),
        ]);
        let m = CommandMap::from_yscm(&t);
        assert_eq!(m.canon(0), FOREIGN);
        assert_eq!(m.name(m.canon(0)), "CDDA");
        assert_eq!(m.canon(1), crate::cmd::END);
        assert_eq!(m.canon(2), crate::cmd::VARINFO);
        assert_eq!(m.foreign_names(), vec!["CDDA"]);
        let changes = m.slot_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].1, vec![(15, 17), (16, 18), (17, 19), (18, 20)]);
    }

    #[test]
    fn canonical_matches_cmd_constants() {
        use crate::cmd;
        for (code, name) in [
            (cmd::GO, "GO"),
            (cmd::GOSUB, "GOSUB"),
            (cmd::IF, "IF"),
            (cmd::ELSE, "ELSE"),
            (cmd::END, "END"),
            (cmd::LET, "LET"),
            (cmd::RETURN, "RETURN"),
            (cmd::TEXT, "TEXT"),
            (cmd::WAIT, "WAIT"),
            (cmd::TASK, "TASK"),
        ] {
            assert_eq!(CANONICAL[code as usize], name);
        }
    }
}
