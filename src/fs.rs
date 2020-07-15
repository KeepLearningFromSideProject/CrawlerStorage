use crate::{
    models::{self, Comic, Eposide, File},
    schema,
};
use diesel::prelude::*;
use fuse::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyWrite, Request,
};
use libc::{EEXIST, EIO, EISDIR, ENOENT, ENOSYS, ENOTDIR, EPERM};
use sha2::{Digest, Sha256};
use std::{
    convert::{TryFrom, TryInto},
    env,
    ffi::OsStr,
    fmt, fs,
    os::unix::fs::{FileExt, MetadataExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub enum InodeKind {
    File,
    Eposide,
    Comic,
    Tag,
    Special,
}

#[derive(Copy, Clone, Hash, PartialEq, Eq)]
pub struct Inode(u64);

impl From<u64> for Inode {
    fn from(ino: u64) -> Self {
        Inode(ino)
    }
}

impl fmt::Debug for Inode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inode")
            .field("value", &self.0)
            .field("_kind", &self.kind())
            .field("_id", &self.id())
            .finish()
    }
}

impl Inode {
    pub const IS_FILE: u64 = 1 << 63;
    pub const IS_EPOSIDE: u64 = 1 << 62;
    pub const IS_COMIC: u64 = 1 << 61;
    pub const IS_TAG: u64 = 1 << 60;
    pub const MARK_MASK: u64 = Self::IS_COMIC | Self::IS_EPOSIDE | Self::IS_FILE | Self::IS_TAG;
    pub const NODE_MASK: u64 = !Self::MARK_MASK;

    pub fn kind(self) -> InodeKind {
        if self.is_file() {
            InodeKind::File
        } else if self.is_eposide() {
            InodeKind::Eposide
        } else if self.is_comic() {
            InodeKind::Comic
        } else if self.is_tag() {
            InodeKind::Tag
        } else {
            InodeKind::Special
        }
    }

    pub fn is_file(self) -> bool {
        self.0 & Self::IS_FILE != 0
    }

    pub fn is_eposide(self) -> bool {
        self.0 & Self::IS_EPOSIDE != 0
    }

    pub fn is_comic(self) -> bool {
        self.0 & Self::IS_COMIC != 0
    }

    pub fn is_tag(self) -> bool {
        self.0 & Self::IS_TAG != 0
    }

    pub fn is_special(self) -> bool {
        self.0 & Self::MARK_MASK == 0
    }

    pub fn child_kind(self) -> Option<InodeKind> {
        let kind = self.kind();
        match kind {
            InodeKind::Comic => Some(InodeKind::Eposide),
            InodeKind::Eposide => Some(InodeKind::File),
            _ => None,
        }
    }

    pub fn id(self) -> u64 {
        self.0 & Self::NODE_MASK
    }
}

impl Inode {
    fn comic(id: i32) -> Self {
        Self(Self::IS_COMIC | u64::try_from(id).unwrap())
    }

    fn eposide(id: i32) -> Self {
        Self(Self::IS_EPOSIDE | u64::try_from(id).unwrap())
    }

    fn file(id: i32) -> Self {
        Self(Self::IS_FILE | u64::try_from(id).unwrap())
    }
}

pub struct ComicFS {
    conn: SqliteConnection,
}

static ONE_SEC: Duration = Duration::from_secs(1);

static ROOT_DIR_ATTR: FileAttr = FileAttr {
    ino: 1,
    size: 0,
    blocks: 0,
    atime: SystemTime::UNIX_EPOCH, // 1970-01-01 00:00:00
    mtime: SystemTime::UNIX_EPOCH,
    ctime: SystemTime::UNIX_EPOCH,
    crtime: SystemTime::UNIX_EPOCH,
    kind: FileType::Directory,
    perm: 0o755,
    nlink: 2,
    uid: 1000,
    gid: 1000,
    rdev: 0,
    flags: 0,
};

static SPECIAL_DIR_ATTRS: [FileAttr; 2] = [
    FileAttr {
        ino: 2,
        size: 0,
        blocks: 0,
        atime: SystemTime::UNIX_EPOCH, // 1970-01-01 00:00:00
        mtime: SystemTime::UNIX_EPOCH,
        ctime: SystemTime::UNIX_EPOCH,
        crtime: SystemTime::UNIX_EPOCH,
        kind: FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
    },
    FileAttr {
        ino: 3,
        size: 0,
        blocks: 0,
        atime: SystemTime::UNIX_EPOCH, // 1970-01-01 00:00:00
        mtime: SystemTime::UNIX_EPOCH,
        ctime: SystemTime::UNIX_EPOCH,
        crtime: SystemTime::UNIX_EPOCH,
        kind: FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
    },
];

fn directory_attr(inode: Inode) -> FileAttr {
    FileAttr {
        ino: inode.0,
        size: 0,
        blocks: 0,
        atime: SystemTime::UNIX_EPOCH,
        mtime: SystemTime::UNIX_EPOCH,
        ctime: SystemTime::UNIX_EPOCH,
        crtime: SystemTime::UNIX_EPOCH,
        kind: FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
    }
}

fn file_attr(inode: Inode) -> FileAttr {
    FileAttr {
        ino: inode.0,
        size: 0,
        blocks: 0,
        atime: SystemTime::UNIX_EPOCH,
        mtime: SystemTime::UNIX_EPOCH,
        ctime: SystemTime::UNIX_EPOCH,
        crtime: SystemTime::UNIX_EPOCH,
        kind: FileType::RegularFile,
        perm: 0o644,
        nlink: 2,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
    }
}

impl ComicFS {
    const ROOT_ID: u64 = 1;
    const COMIC_ID: u64 = 2;
    const TAGS_ID: u64 = 3;

    fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    fn find_comic_by_inode(&self, inode: Inode) -> Option<FileAttr> {
        Comic::find(i32::try_from(inode.id()).unwrap(), &self.conn)
            .map(|info| directory_attr(Inode::comic(info.id)))
    }

    fn find_eposide_by_inode(&self, inode: Inode) -> Option<FileAttr> {
        let res = Eposide::find(i32::try_from(inode.id()).unwrap(), &self.conn);
        res.map(|info| directory_attr(Inode::eposide(info.id)))
    }

    fn find_comic_by_name(&self, name: &str) -> Option<FileAttr> {
        Comic::find_by_name(name, &self.conn).map(|info| directory_attr(Inode::comic(info.id)))
    }

    fn find_comic_eposide_by_name(&self, id: u64, name: &str) -> Option<FileAttr> {
        Eposide::find_by_comic_and_name(i32::try_from(id).unwrap(), name, &self.conn)
            .map(|info| directory_attr(Inode::eposide(info.id)))
    }

    fn find_eposide_file_by_name(&self, id: u64, name: &str) -> Option<FileAttr> {
        File::find_by_eposide_and_name(i32::try_from(id).unwrap(), name, &self.conn)
            .map(|info| file_attr(Inode::file(info.id)))
    }

    fn inode_to_storage(&self, ino: Inode) -> Option<PathBuf> {
        let info = File::find(i32::try_from(ino.id()).unwrap(), &self.conn)?;
        let path = generate_storage_path(&info.content_hash);
        Some(path)
    }
}

impl Filesystem for ComicFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        match parent {
            Self::ROOT_ID => {
                if name == "comics" {
                    reply.entry(&ONE_SEC, &SPECIAL_DIR_ATTRS[0], 0);
                } else if name == "tags" {
                    reply.entry(&ONE_SEC, &SPECIAL_DIR_ATTRS[1], 0);
                } else {
                    reply.error(ENOENT);
                }
            }
            Self::COMIC_ID => {
                let name = name.to_str().unwrap();
                let attr = self.find_comic_by_name(name);
                match attr {
                    Some(attr) => {
                        reply.entry(&ONE_SEC, &attr, 0);
                    }
                    None => {
                        reply.error(ENOENT);
                    }
                }
            }
            Self::TAGS_ID => {
                reply.error(ENOENT);
            }
            ino => {
                let ino = Inode::from(ino);
                let kind = ino.kind();
                let attr = match kind {
                    InodeKind::Comic => {
                        let name = name.to_str().unwrap();
                        self.find_comic_eposide_by_name(ino.id(), name)
                    }
                    InodeKind::Eposide => {
                        let name = name.to_str().unwrap();
                        let info = File::find_by_eposide_and_name(
                            i32::try_from(ino.id()).unwrap(),
                            name,
                            &self.conn,
                        );
                        info.and_then(|info| {
                            let id = info.id;
                            if info.content_hash == "" {
                                return Some(file_attr(Inode::file(id)));
                            }
                            let path = generate_storage_path(&info.content_hash);
                            let meta = fs::metadata(&path).ok()?;
                            Some(convert_meta_to_attr(Inode::file(id).0, meta))
                        })
                    }
                    InodeKind::Special | InodeKind::File => unreachable!(),
                    _ => todo!(),
                };

                match attr {
                    Some(attr) => {
                        reply.entry(&ONE_SEC, &attr, 0);
                    }
                    None => {
                        reply.error(ENOENT);
                    }
                }
            }
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        match ino {
            Self::ROOT_ID => reply.attr(&ONE_SEC, &ROOT_DIR_ATTR),
            Self::COMIC_ID => reply.attr(&ONE_SEC, &SPECIAL_DIR_ATTRS[0]),
            Self::TAGS_ID => reply.attr(&ONE_SEC, &SPECIAL_DIR_ATTRS[1]),
            ino => {
                let ino = Inode::from(ino);
                let kind = ino.kind();
                let attr = match kind {
                    InodeKind::Comic => self.find_comic_by_inode(ino),
                    InodeKind::Eposide => self.find_eposide_by_inode(ino),
                    InodeKind::File => {
                        let info = File::find(i32::try_from(ino.id()).unwrap(), &self.conn);
                        info.and_then(|info| {
                            let id = info.id;
                            if info.content_hash == "" {
                                return Some(file_attr(Inode::file(id)));
                            }
                            let path = generate_storage_path(&info.content_hash);
                            let meta = fs::metadata(&path).ok()?;
                            Some(convert_meta_to_attr(Inode::file(id).0, meta))
                        })
                    }
                    _ => todo!(),
                };
                match attr {
                    Some(attr) => {
                        reply.attr(&ONE_SEC, &attr);
                    }
                    None => {
                        reply.error(ENOENT);
                    }
                }
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        match ino {
            Self::ROOT_ID => {
                if offset == 0 {
                    reply.add(1, 1, FileType::Directory, ".");
                    reply.add(1, 2, FileType::Directory, "..");
                    reply.add(2, 3, FileType::Directory, "comics");
                    reply.add(3, 4, FileType::Directory, "tags");
                }
            }
            Self::COMIC_ID => {
                use schema::comics::dsl;
                if offset == 0 {
                    let comics = dsl::comics.load::<Comic>(&self.conn);
                    if let Ok(comics) = comics {
                        for (i, comic) in comics.iter().enumerate() {
                            let ino = Inode::comic(comic.id);
                            reply.add(
                                ino.0,
                                (i + 1).try_into().unwrap(),
                                FileType::Directory,
                                &comic.name,
                            );
                        }
                    }
                }
            }
            Self::TAGS_ID => (),
            ino => {
                let ino = Inode::from(ino);
                let kind = ino.kind();
                match kind {
                    InodeKind::Comic => {
                        use schema::eposides::dsl;
                        if offset == 0 {
                            let eposides = dsl::eposides
                                .filter(dsl::comic_id.eq(i32::try_from(ino.id()).unwrap()))
                                .load::<Eposide>(&self.conn);
                            if let Ok(eposides) = eposides {
                                for (i, eposide) in eposides.iter().enumerate() {
                                    let ino = Inode::eposide(eposide.id);
                                    reply.add(
                                        ino.0,
                                        (i + 1).try_into().unwrap(),
                                        FileType::Directory,
                                        &eposide.name,
                                    );
                                }
                            }
                        }
                    }
                    InodeKind::Eposide => {
                        use schema::files::dsl;
                        if offset == 0 {
                            let files = dsl::files
                                .filter(dsl::eposid_id.eq(i32::try_from(ino.id()).unwrap()))
                                .load::<File>(&self.conn);
                            if let Ok(files) = files {
                                for (i, file) in files.iter().enumerate() {
                                    let ino = Inode::file(file.id);
                                    reply.add(
                                        ino.0,
                                        (i + 1).try_into().unwrap(),
                                        FileType::RegularFile,
                                        &file.name,
                                    );
                                }
                            }
                        }
                    }
                    InodeKind::File => unreachable!(),
                    _ => todo!(),
                }
            }
        }
        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        reply: ReplyData,
    ) {
        let ino = Inode::from(ino);
        let kind = ino.kind();
        if kind != InodeKind::File {
            reply.error(EISDIR);
            return;
        }
        let path = self.inode_to_storage(ino);
        let path = match path {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        let file = fs::File::open(&path);
        match file {
            Ok(file) => {
                let mut buf = vec![0; usize::try_from(size).unwrap()];
                match file.read_at(&mut buf, u64::try_from(offset).unwrap()) {
                    Ok(size) => {
                        reply.data(&buf[0..size]);
                    }
                    Err(_) => {
                        reply.error(EIO);
                    }
                }
            }
            Err(_) => {
                // TODO: decide to return error or empty content
                reply.data(&[]);
            }
        }
    }

    fn mkdir(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        reply: ReplyEntry,
    ) {
        let parent = Inode::from(parent);
        let kind = parent.kind();
        match kind {
            InodeKind::Special => {
                match parent.0 {
                    1 => {
                        reply.error(EPERM);
                    }
                    // comics
                    2 => {
                        let name = name.to_str().unwrap();
                        let comic = models::NewComic { name };
                        let comic = self
                            .conn
                            .transaction::<_, diesel::result::Error, _>(|| {
                                use schema::comics::dsl;

                                dbg!(diesel::insert_into(dsl::comics)
                                    .values(&comic)
                                    .execute(&self.conn)?);

                                Ok(dsl::comics
                                    .order(dsl::id.desc())
                                    .first::<models::Comic>(&self.conn)?)
                            })
                            .expect("Fail to insert");
                        let ino = Inode::comic(comic.id);
                        reply.entry(&ONE_SEC, &directory_attr(ino), 0);
                    }
                    3 => todo!(),
                    _ => unreachable!(),
                }
            }
            InodeKind::Comic => {
                let name = name.to_str().unwrap();
                let eposide = models::NewEposide {
                    name,
                    comic_id: i32::try_from(parent.id()).unwrap(),
                };
                let eposide = self
                    .conn
                    .transaction::<_, diesel::result::Error, _>(|| {
                        use schema::eposides::dsl;

                        dbg!(diesel::insert_into(dsl::eposides)
                            .values(&eposide)
                            .execute(&self.conn)?);

                        Ok(dsl::eposides
                            .order(dsl::id.desc())
                            .first::<models::Eposide>(&self.conn)?)
                    })
                    .expect("Fail to insert");
                let ino = Inode::eposide(eposide.id);
                reply.entry(&ONE_SEC, &directory_attr(ino), 0);
            }
            InodeKind::Eposide | InodeKind::Tag => {
                reply.error(EPERM);
            }
            InodeKind::File => {
                reply.error(ENOTDIR);
            }
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _flags: u32,
        reply: ReplyCreate,
    ) {
        let parent = Inode::from(parent);
        if parent.kind() != InodeKind::Eposide {
            reply.error(EPERM);
            return;
        }
        let name = name.to_str().unwrap();
        let value = models::NewFile {
            name,
            eposid_id: i32::try_from(parent.id()).unwrap(),
            content_hash: "",
        };
        let file = value.insert(&self.conn).unwrap();
        let ino = Inode::from(u64::try_from(file.id).unwrap());
        reply.created(&ONE_SEC, &file_attr(ino), 0, 0, 0);
    }

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<SystemTime>,
        mtime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        todo!()
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _flags: u32,
        reply: ReplyWrite,
    ) {
        let ino = Inode::from(ino);
        if ino.kind() != InodeKind::File {
            reply.error(EISDIR);
            return;
        }
        let info = match File::find(i32::try_from(ino.id()).unwrap(), &self.conn) {
            Some(info) => info,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        let file = if info.content_hash == "" {
            let hash = Sha256::digest(data);
            let res = hex::encode(&hash);
            let path = generate_storage_path(&res);
            fs::File::create(&path).unwrap()
        } else {
            let path = generate_storage_path(&info.content_hash);
            fs::OpenOptions::new().write(true).open(&path).unwrap()
        };
        let res = file.write_at(data, u64::try_from(offset).unwrap()).unwrap();
        reply.written(u32::try_from(res).unwrap());
    }
}

fn convert_nix_error(err: nix::Error) -> i32 {
    match err {
        nix::Error::Sys(errno) => errno as i32,
        _ => EIO,
    }
}

fn convert_meta_to_attr(ino: u64, meta: fs::Metadata) -> fuse::FileAttr {
    FileAttr {
        ino,
        size: meta.len(),
        nlink: 1,
        perm: cast::u16(meta.mode()).unwrap(),
        uid: meta.uid(),
        gid: meta.gid(),
        blocks: meta.blocks(),
        atime: meta.accessed().unwrap(),
        ctime: meta.created().unwrap(),
        mtime: meta.modified().unwrap(),
        crtime: SystemTime::UNIX_EPOCH,
        kind: convert_file_type(meta.file_type()),
        rdev: 0,
        flags: 0,
    }
}

fn generate_storage_path(content_hash: &str) -> PathBuf {
    let mut path = env::current_dir().unwrap();
    path.push("storage");
    path.push(&content_hash[0..2]);
    path.push(&content_hash);
    path
}

fn convert_file_type(kind: fs::FileType) -> fuse::FileType {
    if kind.is_dir() {
        fuse::FileType::Directory
    } else if kind.is_file() {
        fuse::FileType::RegularFile
    } else if kind.is_symlink() {
        fuse::FileType::Symlink
    } else {
        todo!()
    }
}

pub fn mount(conn: SqliteConnection, mountpoint: &OsStr) {
    let options = ["-o", "rw", "-o", "fsname=comic"]
        .iter()
        .map(|o| o.as_ref())
        .collect::<Vec<&OsStr>>();
    fuse::mount(ComicFS::new(conn), mountpoint, &options).unwrap();
}

#[cfg(test)]
mod tests {
    use super::Inode;

    #[test]
    fn test_inode_is_special() {
        let inode = Inode(1);
        assert!(inode.is_special());
    }
}
