use crate::hex::Hex;
use crate::{
    models::{self, Comic, Episode, File, NewTag, Tag, Taggable, Taggables},
    schema,
};
use diesel::prelude::*;
use fuse::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyWrite, Request,
};
use libc::{EINVAL, EIO, EISDIR, ENOENT, ENOSYS, ENOTDIR, EPERM};
use nix::{
    fcntl::{open, OFlag},
    sys::stat::Mode,
    unistd::{close, ftruncate},
};
use once_cell::sync::Lazy;
use path_clean::PathClean;
use sha2::{Digest, Sha256};
use std::{
    convert::{TryFrom, TryInto},
    env,
    ffi::{CString, OsStr},
    fmt, fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{FileExt, MetadataExt},
    },
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tracing::{info, info_span};

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub enum InodeKind {
    File,
    Eposide,
    Comic,
    Tag,
    Tagged,
    Special,
}

static STORAGE_BASE: Lazy<PathBuf> = Lazy::new(|| {
    let mut cwd = env::current_dir().unwrap();
    let path = env::var_os("FILES_PATH").unwrap();
    cwd.push(path);
    cwd
});

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
            .field("value", &Hex(self.0))
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
    pub const IS_TAGGED: u64 = 1 << 59;
    pub const MARK_MASK: u64 =
        Self::IS_COMIC | Self::IS_EPOSIDE | Self::IS_FILE | Self::IS_TAG | Self::IS_TAGGED;
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
        } else if self.is_tagged() {
            InodeKind::Tagged
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

    pub fn is_tagged(self) -> bool {
        self.0 & Self::IS_TAGGED != 0
    }

    pub fn is_special(self) -> bool {
        self.0 & Self::MARK_MASK == 0
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

    fn tag(id: i32) -> Self {
        Self(Self::IS_TAG | u64::try_from(id).unwrap())
    }

    fn tagged(id: i32) -> Self {
        Self(Self::IS_TAGGED | u64::try_from(id).unwrap())
    }
}

#[derive(derive_more::DebugCustom)]
#[debug(fmt = "ComicFS {{ base: {:?} }}", base)]
pub struct ComicFS {
    conn: SqliteConnection,
    base: PathBuf,
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

fn symlink_attr(inode: Inode, size: u64) -> FileAttr {
    FileAttr {
        ino: inode.0,
        size,
        blocks: 0,
        atime: SystemTime::UNIX_EPOCH,
        mtime: SystemTime::UNIX_EPOCH,
        ctime: SystemTime::UNIX_EPOCH,
        crtime: SystemTime::UNIX_EPOCH,
        kind: FileType::Symlink,
        perm: 0o755,
        nlink: 1,
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

    fn new(conn: SqliteConnection, base: PathBuf) -> Self {
        Self { conn, base }
    }

    fn find_comic_by_inode(&self, inode: Inode) -> Option<FileAttr> {
        Comic::find(i32::try_from(inode.id()).unwrap(), &self.conn)
            .map(|info| directory_attr(Inode::comic(info.id)))
    }

    fn find_eposide_by_inode(&self, inode: Inode) -> Option<FileAttr> {
        let res = Episode::find(i32::try_from(inode.id()).unwrap(), &self.conn);
        res.map(|info| directory_attr(Inode::eposide(info.id)))
    }

    fn find_comic_by_name(&self, name: &str) -> Option<FileAttr> {
        Comic::find_by_name(name, &self.conn).map(|info| directory_attr(Inode::comic(info.id)))
    }

    fn find_comic_eposide_by_name(&self, id: u64, name: &str) -> Option<FileAttr> {
        Episode::find_by_comic_and_name(i32::try_from(id).unwrap(), name, &self.conn)
            .map(|info| directory_attr(Inode::eposide(info.id)))
    }

    fn find_tag_by_name(&self, name: &str) -> Option<FileAttr> {
        Tag::find_by_name(name, &self.conn).map(|info| directory_attr(Inode::tag(info.id)))
    }

    fn inode_to_storage(&self, ino: Inode) -> Option<PathBuf> {
        let info = File::find(i32::try_from(ino.id()).unwrap(), &self.conn)?;
        let path = generate_storage_path(&info.content_hash);
        Some(path)
    }

    fn resolve(&self, path: &Path) -> Option<Inode> {
        let mut parent = Inode::from(1);
        for component in path.components() {
            let id = parent.0;
            let kind = parent.kind();
            let name = component.as_os_str();

            match kind {
                InodeKind::Special => match id {
                    Self::ROOT_ID => {
                        if name == "comics" {
                            parent = Inode::from(Self::COMIC_ID);
                        } else if name == "tags" {
                            parent = Inode::from(Self::TAGS_ID);
                        } else {
                            unreachable!();
                        }
                    }
                    Self::COMIC_ID => {
                        let info = Comic::find_by_name(name.to_str().unwrap(), &self.conn)?;
                        parent = Inode::comic(info.id);
                    }
                    Self::TAGS_ID => {
                        let info = Tag::find_by_name(name.to_str().unwrap(), &self.conn)?;
                        parent = Inode::tag(info.id)
                    }
                    _ => unreachable!(),
                },
                InodeKind::Comic => {
                    let info = Episode::find_by_comic_and_name(
                        parent.id().try_into().unwrap(),
                        name.to_str().unwrap(),
                        &self.conn,
                    )?;
                    parent = Inode::eposide(info.id);
                }
                InodeKind::Eposide => {
                    let info = File::find_by_eposide_and_name(
                        parent.id().try_into().unwrap(),
                        name.to_str().unwrap(),
                        &self.conn,
                    )?;
                    parent = Inode::file(info.id);
                }
                InodeKind::Tag => todo!(),
                InodeKind::File => {
                    unreachable!();
                }
                InodeKind::Tagged => {
                    unreachable!();
                }
            }
        }
        Some(parent)
    }

    fn resolve_inode(&self, ino: Inode) -> Option<PathBuf> {
        let mut next = Some(ino);
        let mut components = vec![];

        while let Some(ino) = next {
            match ino.kind() {
                InodeKind::Special => match ino.0 {
                    Self::ROOT_ID => {
                        components.push(self.base.clone());
                        next = None;
                    }
                    Self::COMIC_ID => {
                        components.push(PathBuf::from("comics".to_owned()));
                        next = Some(Inode::from(Self::ROOT_ID));
                    }
                    Self::TAGS_ID => {
                        components.push(PathBuf::from("tags".to_owned()));
                        next = Some(Inode::from(Self::ROOT_ID));
                    }
                    _ => unreachable!(),
                },
                InodeKind::Comic => {
                    let info = Comic::find(ino.id().try_into().unwrap(), &self.conn)?;
                    components.push(PathBuf::from(info.name.clone()));
                    next = Some(Inode::from(Self::COMIC_ID));
                }
                InodeKind::Eposide => {
                    let info = Episode::find(ino.id().try_into().unwrap(), &self.conn)?;
                    components.push(PathBuf::from(info.name.clone()));
                    next = Some(Inode::comic(info.comic_id));
                }
                InodeKind::File => {
                    let info = File::find(ino.id().try_into().unwrap(), &self.conn)?;
                    components.push(PathBuf::from(info.name.clone()));
                    next = Some(Inode::eposide(info.eposid_id));
                }
                InodeKind::Tag => {
                    let info = Tag::find(ino.id().try_into().unwrap(), &self.conn)?;
                    components.push(PathBuf::from(info.name.clone()));
                    next = Some(Inode::from(Self::TAGS_ID));
                }
                _ => todo!(),
            }
        }

        Some(components.into_iter().rev().collect::<PathBuf>())
    }
}

impl Filesystem for ComicFS {
    #[tracing::instrument(fields(unique = _req.unique()),skip(self, _req,  reply))]
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
                let name = name.to_str().unwrap();
                let attr = self.find_tag_by_name(name);
                match attr {
                    Some(attr) => {
                        reply.entry(&ONE_SEC, &attr, 0);
                    }
                    None => {
                        reply.error(ENOENT);
                    }
                }
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
                    InodeKind::Special | InodeKind::File | InodeKind::Tagged => unreachable!(),
                    InodeKind::Tag => {
                        let span = info_span!("lookop tagged");
                        let _guard = span.enter();
                        let expected_name = name.to_str().unwrap();
                        info!(expected_name);
                        let files =
                            Taggables::taggables(i32::try_from(ino.id()).unwrap(), &self.conn);
                        info!(?files);
                        let res = files.iter().find_map(|file| match file {
                            Taggables::Comic { id, name, .. } => {
                                if name == expected_name {
                                    let id = *id;
                                    info!(id, "found comic");
                                    let path = self
                                        .resolve_inode(Inode::comic(id.try_into().unwrap()))
                                        .unwrap();
                                    Some((id, path))
                                } else {
                                    None
                                }
                            }
                            Taggables::Episode { id, name, .. } => {
                                if name == expected_name {
                                    let id = *id;
                                    info!(id, "found episode");
                                    let path = self
                                        .resolve_inode(Inode::eposide(id.try_into().unwrap()))
                                        .unwrap();
                                    Some((id, path))
                                } else {
                                    None
                                }
                            }
                            Taggables::File { id, name, .. } => {
                                if name == expected_name {
                                    let id = *id;
                                    info!(id, "found file");
                                    let path = self
                                        .resolve_inode(Inode::file(id.try_into().unwrap()))
                                        .unwrap();
                                    Some((id, path))
                                } else {
                                    None
                                }
                            }
                        });
                        let (id, path) = match res {
                            Some(id) => id,
                            None => {
                                info!("not found");
                                reply.error(ENOENT);
                                return;
                            }
                        };
                        let ino = Inode::tagged(id);
                        Some(symlink_attr(ino, path.as_os_str().len() as u64))
                    }
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

    #[tracing::instrument(fields(unique = _req.unique(), ino = ?Inode::from(ino)),skip(self, _req, ino, reply))]
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
                    InodeKind::Tag => {
                        let info = Tag::find(i32::try_from(ino.id()).unwrap(), &self.conn);
                        info.map(|info| {
                            let ino = Inode::tag(info.id);
                            directory_attr(ino)
                        })
                    }
                    InodeKind::Tagged => {
                        let info = Taggable::find(i32::try_from(ino.id()).unwrap(), &self.conn);
                        info!(?info);
                        info.map(|info| {
                            let target = match info.taggable_type.as_str() {
                                "comic" => Inode::comic(info.taggable_id),
                                "eposide" => Inode::eposide(info.taggable_id),
                                "file" => Inode::file(info.taggable_id),
                                _ => unreachable!(),
                            };
                            let path = self.resolve_inode(target).unwrap();
                            let len = path.as_os_str().len();
                            assert_eq!(len, path.as_os_str().as_bytes().len());
                            let attr = symlink_attr(ino, len as u64);
                            info!(?attr);
                            attr
                        })
                    }
                    InodeKind::Special => unreachable!(),
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

    #[tracing::instrument(fields(unique = _req.unique(), ino = ?Hex(ino)),skip(self, _req, ino, _fh, reply))]
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
            Self::TAGS_ID => {
                if offset == 0 {
                    if let Some(tags) = Tag::list(&self.conn) {
                        for (i, tag) in tags.iter().enumerate() {
                            let ino = Inode::tag(tag.id);
                            reply.add(
                                ino.0,
                                (i + 1).try_into().unwrap(),
                                FileType::Directory,
                                &tag.name,
                            );
                        }
                    }
                }
            }
            ino => {
                let ino = Inode::from(ino);
                let kind = ino.kind();
                match kind {
                    InodeKind::Comic => {
                        use schema::eposides::dsl;
                        if offset == 0 {
                            let eposides = dsl::eposides
                                .filter(dsl::comic_id.eq(i32::try_from(ino.id()).unwrap()))
                                .load::<Episode>(&self.conn);
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
                    InodeKind::Tag if offset == 0 => {
                        let taggables =
                            Taggables::taggables(ino.id().try_into().unwrap(), &self.conn);
                        for (i, taggable) in taggables.iter().enumerate() {
                            match taggable {
                                Taggables::Comic { id, name, .. } => {
                                    let ino = Inode::tagged(*id);
                                    reply.add(
                                        ino.0,
                                        (i + 1).try_into().unwrap(),
                                        FileType::Symlink,
                                        &name,
                                    );
                                }
                                Taggables::Episode { id, name, .. } => {
                                    let ino = Inode::tagged(*id);
                                    reply.add(
                                        ino.0,
                                        (i + 1).try_into().unwrap(),
                                        FileType::Symlink,
                                        &name,
                                    );
                                }
                                Taggables::File { id, name, .. } => {
                                    let ino = Inode::tagged(*id);
                                    reply.add(
                                        ino.0,
                                        (i + 1).try_into().unwrap(),
                                        FileType::Symlink,
                                        &name,
                                    );
                                }
                            }
                        }
                    }
                    InodeKind::Tag => (),
                    InodeKind::File | InodeKind::Special | InodeKind::Tagged => unreachable!(),
                }
            }
        }
        reply.ok();
    }

    #[tracing::instrument(fields(unique = _req.unique(), ino = ?Hex(ino)),skip(self, _req, ino, _fh, reply))]
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
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
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

                                diesel::insert_into(dsl::comics)
                                    .values(&comic)
                                    .execute(&self.conn)?;

                                Ok(dsl::comics
                                    .order(dsl::id.desc())
                                    .first::<models::Comic>(&self.conn)?)
                            })
                            .expect("Fail to insert");
                        let ino = Inode::comic(comic.id);
                        reply.entry(&ONE_SEC, &directory_attr(ino), 0);
                    }
                    3 => {
                        let name = name.to_str().unwrap();
                        let tag = NewTag { name };
                        let tag = tag.insert(&self.conn).expect("Fail to insert tag");
                        let ino = Inode::tag(tag.id);
                        reply.entry(&ONE_SEC, &directory_attr(ino), 0);
                    }
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

                        diesel::insert_into(dsl::eposides)
                            .values(&eposide)
                            .execute(&self.conn)?;

                        Ok(dsl::eposides
                            .order(dsl::id.desc())
                            .first::<models::Episode>(&self.conn)?)
                    })
                    .expect("Fail to insert");
                let ino = Inode::eposide(eposide.id);
                reply.entry(&ONE_SEC, &directory_attr(ino), 0);
            }
            InodeKind::Eposide | InodeKind::Tag => {
                reply.error(EPERM);
            }
            InodeKind::File | InodeKind::Tagged => {
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
        let ino = Inode::file(file.id);
        reply.created(&ONE_SEC, &file_attr(ino), 0, 0, 0);
    }

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<SystemTime>,
        _mtime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        let ino = Inode::from(ino);
        if ino.kind() != InodeKind::File {
            reply.error(ENOSYS);
            return;
        }
        let info = match File::find(i32::try_from(ino.id()).unwrap(), &self.conn) {
            Some(info) => info,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        if info.content_hash == "" {
            reply.attr(&ONE_SEC, &file_attr(ino));
            return;
        }
        let path = generate_storage_path(&info.content_hash);
        let fd = match open(&path, OFlag::O_WRONLY, Mode::empty()) {
            Ok(fd) => fd,
            Err(err) => {
                reply.error(convert_nix_error(err));
                return;
            }
        };
        scopeguard::defer! {
            let _ = close(fd);
        }
        if let Some(size) = size {
            if let Err(err) = ftruncate(fd, i64::try_from(size).unwrap()) {
                reply.error(convert_nix_error(err));
                return;
            }
        }
        let meta = fs::metadata(&path).unwrap();
        let attr = convert_meta_to_attr(ino.0, meta);
        reply.attr(&ONE_SEC, &attr);
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
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            info.update_content_hash(&res, &self.conn);
            fs::File::create(&path).unwrap()
        } else {
            let path = generate_storage_path(&info.content_hash);
            fs::OpenOptions::new().write(true).open(&path).unwrap()
        };
        let res = file.write_at(data, u64::try_from(offset).unwrap()).unwrap();
        reply.written(u32::try_from(res).unwrap());
    }

    #[tracing::instrument(fields(unique = _req.unique(), ino = ?Hex(ino)),skip(self, _req, ino, reply))]
    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        let ino = Inode::from(ino);
        if ino.kind() != InodeKind::Tagged {
            reply.error(EINVAL);
            return;
        }
        let info = match Taggable::find_info(ino.id().try_into().unwrap(), &self.conn) {
            Some(info) => info,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        info!(?ino, ?info);
        let ino = match info {
            Taggables::Comic { comic, .. } => Inode::comic(comic.id),
            Taggables::Episode { episode, .. } => Inode::eposide(episode.id),
            Taggables::File { file, .. } => Inode::file(file.id),
        };
        let path = self.resolve_inode(ino).unwrap();
        info!(path = %path.display(), path.len = path.as_os_str().len());
        let bytes = path.as_os_str().as_bytes();
        reply.data(bytes);
    }

    fn link(
        &mut self,
        _req: &Request,
        ino: u64,
        newparent: u64,
        _newname: &OsStr,
        reply: ReplyEntry,
    ) {
        let ino = Inode::from(ino);
        let tag_ino = Inode::from(newparent);
        match ino.kind() {
            InodeKind::Special | InodeKind::Tag => {
                reply.error(EPERM);
                return;
            }
            InodeKind::Comic => {
                Taggable::comic(
                    ino.id().try_into().unwrap(),
                    tag_ino.id().try_into().unwrap(),
                    &self.conn,
                )
                .unwrap();
                reply.entry(&ONE_SEC, &directory_attr(ino), 0);
            }
            InodeKind::Eposide => {
                todo!();
            }
            InodeKind::File => {
                todo!();
            }
            InodeKind::Tagged => unreachable!(),
        }
    }

    fn symlink(
        &mut self,
        _req: &Request,
        parent: u64,
        _name: &OsStr,
        link: &Path,
        reply: ReplyEntry,
    ) {
        let tag_ino = Inode::from(parent);
        if tag_ino.kind() != InodeKind::Tag {
            reply.error(EPERM);
            return;
        }
        let path = if link.is_absolute() {
            link.to_owned()
        } else {
            let path = match self.resolve_inode(Inode::from(parent)) {
                Some(path) => path,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            };
            path.join(link).clean()
        };
        let target = dbg!(path.strip_prefix(&self.base).unwrap());
        let ino = match self.resolve(target) {
            Some(ino) => ino,
            None => {
                reply.error(EPERM);
                return;
            }
        };
        match ino.kind() {
            InodeKind::Special | InodeKind::Tag | InodeKind::Tagged => {
                reply.error(EPERM);
                return;
            }
            InodeKind::Comic => {
                let info = Taggable::comic(
                    ino.id().try_into().unwrap(),
                    tag_ino.id().try_into().unwrap(),
                    &self.conn,
                )
                .unwrap();

                let path = self
                    .resolve_inode(Inode::comic(info.id.try_into().unwrap()))
                    .unwrap();
                reply.entry(
                    &ONE_SEC,
                    &symlink_attr(Inode::tagged(info.id), path.as_os_str().len() as u64),
                    0,
                );
            }
            InodeKind::Eposide => {
                todo!();
            }
            InodeKind::File => {
                todo!();
            }
        }
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
    let mut path = STORAGE_BASE.clone();
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
        unreachable!("doesn't support other file kind")
    }
}

pub fn mount(conn: SqliteConnection, mountpoint: &OsStr) {
    let options = ["-o", "rw", "-o", "fsname=comic"]
        .iter()
        .map(|o| o.as_ref())
        .collect::<Vec<&OsStr>>();
    fuse::mount(
        ComicFS::new(
            conn,
            fs::canonicalize(mountpoint).expect("Fail to resolve mount point"),
        ),
        mountpoint,
        &options,
    )
    .unwrap();
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
