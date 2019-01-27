use std::env;
use std::io::Write;
use std::io;
use std::fs;

use byteorder::LE;
use byteorder::WriteBytesExt;
use cast::u64;
use failure::err_msg;
use failure::Error;
use failure::bail;
use failure::ResultExt;
use splayers::Entry;
use splayers::Status;
use failure::ensure;

fn main() -> Result<(), Error> {
    let src = env::args().nth(1).ok_or(err_msg("first arg: src"))?;
    let dest = env::args_os().nth(2).ok_or(err_msg("second arg: dest"))?;
    let mut cwd = env::current_dir()?;
    cwd.push(dest);
    let dest = cwd;
    let root = dest.parent().ok_or(err_msg("file output please"))?;

    let mut tmp = tempfile::NamedTempFile::new()?;
    http_req::request::get(src, &mut tmp)
        .with_context(|_| err_msg("downloading"))?;

    let unpack = splayers::Unpack::unpack_into(tmp.path(), root)
        .with_context(|_| err_msg("unpacking failed"))?;

    let mut out = tempfile_fast::PersistableTempFile::new_in(root)?;

    match *unpack.status() {
        splayers::Status::Success(ref entries) => output(entries, &[], &mut out)?,
        ref other => bail!("expecting top level archive, not: {:?}", other),
    }

    out.persist_noclobber(dest).map_err(|e| e.error)?;

    Ok(())
}

fn output(entries: &[Entry], paths: &[Box<[u8]>], mut out: &mut tempfile_fast::PersistableTempFile) -> Result<(), Error> {
    let mut entries: Vec<&Entry> = entries.iter().collect();

    let mut name_prefix = Vec::with_capacity(paths.len() * 128);
    for path in paths {
        name_prefix.extend_from_slice(path);
        name_prefix.push(0);
    }

    entries.sort_by_key(|e| e.local.path.as_ref());

    for entry in entries {
        #[cfg(nah)]
        let file = match entry.local.meta.item_type {
            ItemType::RegularFile => true,
            _ => false,
        };

        let mut full_name = name_prefix.to_vec();
        full_name.extend_from_slice(&entry.local.path);

        let data_len = if let Some(temp) = entry.local.temp.as_ref() {
           temp.metadata()?.len()
        } else {
            0
        };

        out.write_u64::<LE>(u64(full_name.len()))?;
        out.write_u64::<LE>(data_len)?;
        out.write_all(&full_name)?;

        if let Some(temp) = entry.local.temp.as_ref() {
            let written = io::copy(&mut fs::File::open(temp)?, &mut out)?;
            ensure!(written == data_len, "short write: expected: {}, actual: {}", data_len, written);
        }

        match &entry.children {
            Status::Success(entries) => {
                let mut paths = paths.to_vec();
                paths.push(entry.local.path.clone());
                output(&entries, &paths, &mut out)?;
            }
            _ => (),
        }
    }
    Ok(())
}
