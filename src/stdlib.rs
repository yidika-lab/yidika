pub const SUBMODULES: &[(&str, &[&str])] = &[
    ("fs",       &["read","write","append","remove","exists","list","is_dir","is_file"]),
    ("sys",      &["env","args","exit","cwd","pid","platform","sleep"]),
    ("json",     &["parse","stringify"]),
    ("datetime", &["now","utc","timestamp","format","parse","year","month","day","hour","minute","second"]),
    ("path",     &["join","dirname","basename","extension","is_absolute"]),
    ("base64",   &["encode","decode"]),
    ("regex",       &["match","find","replace","split"]),
    ("math",     &["cos","sin","sqrt","abs","floor","ceil","round","max","min","pow","rand"]),
    ("time",     &["now","utc","timestamp","format","parse","sleep","year","month","day","hour","minute","second"]),
];

pub fn list_submodules() -> impl Iterator<Item = &'static str> {
    SUBMODULES.iter().map(|(name, _)| *name)
}

pub fn submodule_funcs(name: &str) -> Option<&'static [&'static str]> {
    SUBMODULES.iter().find(|(m, _)| *m == name).map(|(_, f)| *f)
}

pub fn has_func(module: &str, func: &str) -> bool {
    submodule_funcs(module).is_some_and(|funcs| funcs.contains(&func))
}

pub fn is_valid_std_path(path: &[&str]) -> bool {
    path.len() == 3 && path[0] == "std" && has_func(path[1], path[2])
}
