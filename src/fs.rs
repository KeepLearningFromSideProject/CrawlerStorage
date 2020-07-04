use crate::{
    models::{Comic, Eposide, File},
    schema,
};
use diesel::prelude::*;
use fuse::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyStatfs, ReplyWrite, Request,
};
use libc::{EEXIST, EIO, ENOENT, ENOSYS, EPERM};
use std::{
    convert::{TryFrom, TryInto},
    ffi::OsStr,
    fmt, fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
enum InodeKind {
    File,
    Eposide,
    Comic,
    Tag,
    Special,
}

#[derive(Copy, Clone, Hash, PartialEq, Eq)]
pub struct Inode(u64);

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
        nlink: 1,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
    }
}

impl ComicFS {
    fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    fn find_comic_by_inode(&self, inode: Inode) -> Option<FileAttr> {
        use schema::comics::dsl;

        let res = dsl::comics
            .find(i32::try_from(inode.id()).unwrap())
            .first::<Comic>(&self.conn);
        res.ok().map(|info| directory_attr(Inode::comic(info.id)))
    }

    fn find_eposide_by_inode(&self, inode: Inode) -> Option<FileAttr> {
        use schema::eposides::dsl;

        let res = dsl::eposides
            .find(i32::try_from(inode.id()).unwrap())
            .first::<Eposide>(&self.conn);
        res.ok().map(|info| directory_attr(Inode::eposide(info.id)))
    }

    fn find_file_by_inode(&self, inode: Inode) -> Option<FileAttr> {
        use schema::files::dsl;

        let res = dsl::files
            .find(i32::try_from(inode.id()).unwrap())
            .first::<File>(&self.conn);
        res.ok().map(|info| file_attr(Inode::file(info.id)))
    }

    fn find_comic_by_name(&self, name: &str) -> Option<FileAttr> {
        use schema::comics::dsl;

        let res = dsl::comics
            .filter(dsl::name.eq(name))
            .first::<Comic>(&self.conn);
        res.ok().map(|info| directory_attr(Inode::comic(info.id)))
    }

    fn find_comic_eposide_by_name(&self, id: u64, name: &str) -> Option<FileAttr> {
        use schema::eposides::dsl;

        let res = dsl::eposides
            .filter(dsl::comic_id.eq(i32::try_from(id).unwrap()))
            .filter(dsl::name.eq(name))
            .first::<Eposide>(&self.conn);
        res.ok().map(|info| directory_attr(Inode::eposide(info.id)))
    }

    fn find_eposide_file_by_name(&self, id: u64, name: &str) -> Option<FileAttr> {
        use schema::files::dsl;

        let res = dsl::files
            .filter(dsl::eposid_id.eq(i32::try_from(id).unwrap()))
            .filter(dsl::name.eq(name))
            .first::<File>(&self.conn);
        res.ok().map(|info| file_attr(Inode::file(info.id)))
    }
}

impl Filesystem for ComicFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        match parent {
            1 => {
                if name == "comics" {
                    reply.entry(&ONE_SEC, &SPECIAL_DIR_ATTRS[0], 0);
                } else if name == "tags" {
                    reply.entry(&ONE_SEC, &SPECIAL_DIR_ATTRS[1], 0);
                } else {
                    reply.error(ENOENT);
                }
            }
            // comics
            2 => {
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
            3 => {
                reply.error(ENOENT);
            }
            ino => {
                let ino = dbg!(Inode(ino));
                let kind = ino.kind();
                let attr = match kind {
                    InodeKind::Comic => {
                        let name = name.to_str().unwrap();
                        self.find_comic_eposide_by_name(ino.id(), name)
                    }
                    InodeKind::Eposide => {
                        let name = name.to_str().unwrap();
                        self.find_eposide_file_by_name(ino.id(), name)
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
            1 => reply.attr(&ONE_SEC, &ROOT_DIR_ATTR),
            2 => reply.attr(&ONE_SEC, &SPECIAL_DIR_ATTRS[0]),
            3 => reply.attr(&ONE_SEC, &SPECIAL_DIR_ATTRS[1]),
            ino => {
                let ino = Inode(ino);
                let kind = ino.kind();
                let attr = match kind {
                    InodeKind::Comic => self.find_comic_by_inode(ino),
                    InodeKind::Eposide => self.find_eposide_by_inode(ino),
                    InodeKind::File => self.find_file_by_inode(ino),
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
            1 => {
                if offset == 0 {
                    reply.add(1, 1, FileType::Directory, ".");
                    reply.add(1, 2, FileType::Directory, "..");
                    reply.add(2, 3, FileType::Directory, "comics");
                    reply.add(3, 4, FileType::Directory, "tags");
                }
            }
            2 => {
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
            3 => (),
            ino => {
                let ino = Inode(ino);
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
        reply.error(ENOSYS);
    }

    fn mkdir(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        reply: ReplyEntry,
    ) {
        todo!()
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _flags: u32,
        reply: ReplyCreate,
    ) {
        todo!()
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
        todo!()
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
