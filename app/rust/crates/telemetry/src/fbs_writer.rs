use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub fn write_runtime_window(name: &str, bytes: &[u8]) {
    // Atomic replace: write temp file then rename to target.
    let mut target = PathBuf::from("/dev/shm");
    target.push(format!("alegria_{name}.fbs"));

    let mut tmp = target.clone();
    tmp.set_extension("tmp");

    if let Ok(mut f) = fs::File::create(&tmp) {
        if f.write_all(bytes).is_ok() && f.sync_all().is_ok() {
            let _ = fs::rename(&tmp, &target);
            return;
        }
    }
    let _ = fs::remove_file(&tmp);
}
