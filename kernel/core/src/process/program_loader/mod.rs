pub(super) mod elf;
mod shebang;

use self::{
    elf::{ElfHeaders, ElfLoadInfo, load_elf_to_vmar},
    shebang::parse_shebang_line,
};
use crate::{
    fs::{
        file::{AccessMode, FileLike, InodeHandle, InodeType, Permission, StatusFlags},
        vfs::{
            inode::Inode,
            notify,
            path::{FsPath, Path, PathResolver, PerMountFlags},
        },
    },
    prelude::*,
    vm::vmar::Vmar,
};

pub(crate) struct UndetectedExecutable {
    file: Arc<dyn FileLike>,
    script_path: ShebangScriptPath,
}

impl UndetectedExecutable {
    pub(crate) fn open(path: Path, script_path: ShebangScriptPath) -> Result<Self> {
        let file = open_executable_file(path)?;
        Ok(Self { file, script_path })
    }

    pub(crate) fn path(&self) -> &Path {
        self.file.path()
    }

    fn detect_format(self) -> Result<DetectedExecutable> {
        let mut first_page = Box::new([0u8; PAGE_SIZE]);
        let len = self.file.read_bytes_at(0, &mut *first_page)?;

        let Some(interpreter_argv) = parse_shebang_line(&first_page[..len])? else {
            let headers = ElfHeaders::parse(&first_page[..len])?;
            return Ok(DetectedExecutable::Elf {
                file: self.file,
                headers,
            });
        };

        let script_path = match self.script_path {
            ShebangScriptPath::Accessible(path) => path,
            ShebangScriptPath::Inaccessible => {
                return Err(errno!(
                    ENOENT,
                    "the script path is inaccessible after closing close-on-exec file descriptors"
                ));
            }
        };

        Ok(DetectedExecutable::Script {
            interpreter_argv,
            script_path,
        })
    }
}

enum DetectedExecutable {
    Elf {
        file: Arc<dyn FileLike>,
        headers: ElfHeaders,
    },
    Script {
        interpreter_argv: Vec<CString>,
        script_path: CString,
    },
}

fn open_executable_file(path: Path) -> Result<Arc<dyn FileLike>> {
    if path.mount_node().flags().contains(PerMountFlags::NOEXEC) {
        return Err(errno!(
            EACCES,
            "the executable is on a file system mounted with `noexec`"
        ));
    }

    check_executable_inode(path.inode().as_ref())?;

    let file: Arc<dyn FileLike> = Arc::new(InodeHandle::new_unchecked_access(
        path,
        AccessMode::O_RDONLY,
        StatusFlags::empty(),
    )?);
    notify::on_open(file.common());

    Ok(file)
}

pub(crate) enum ShebangScriptPath {
    Accessible(CString),
    Inaccessible,
}

pub(super) struct ProgramToLoad {
    elf_file: Arc<dyn FileLike>,
    elf_headers: ElfHeaders,
    argv: Vec<CString>,
    envp: Vec<CString>,
}

impl ProgramToLoad {
    pub(super) fn from_executable(
        mut executable: UndetectedExecutable,
        path_resolver: &PathResolver,
        mut argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Result<Self> {
        let mut recursive_limit = 5;

        let (elf_file, elf_headers) = loop {
            let (mut new_argv, current_script_path) = match executable.detect_format()? {
                DetectedExecutable::Elf { file, headers } => break (file, headers),
                DetectedExecutable::Script {
                    interpreter_argv,
                    script_path,
                } => (interpreter_argv, script_path),
            };

            if recursive_limit == 0 {
                return Err(errno!(ELOOP, "the recursive limit is reached"));
            }
            recursive_limit -= 1;

            let interpreter_filename = new_argv[0].clone();
            let interpreter = {
                let filename = interpreter_filename.to_str()?;
                let fs_path = FsPath::try_from(filename)?;
                path_resolver.lookup(&fs_path)?
            };

            new_argv.push(current_script_path);
            new_argv.extend(argv.into_iter().skip(1));
            argv = new_argv;
            executable = UndetectedExecutable::open(
                interpreter,
                ShebangScriptPath::Accessible(interpreter_filename),
            )?;
        };

        Ok(Self {
            elf_file,
            elf_headers,
            argv,
            envp,
        })
    }

    pub(super) fn elf_path(&self) -> &Path {
        self.elf_file.path()
    }

    pub(super) fn load_to_vmar(
        self,
        vmar: &Vmar,
        path_resolver: &PathResolver,
    ) -> Result<ElfLoadInfo> {
        load_elf_to_vmar(
            vmar,
            self.elf_file,
            path_resolver,
            self.elf_headers,
            self.argv,
            self.envp,
        )
    }
}

fn check_executable_inode(inode: &dyn Inode) -> Result<()> {
    if inode.type_().is_directory() {
        return Err(errno!(EISDIR, "the inode is a directory"));
    }

    if inode.type_() == InodeType::SymLink {
        return Err(errno!(ELOOP, "the inode is a symbolic link"));
    }

    if !inode.type_().is_regular_file() {
        return Err(errno!(EACCES, "the inode is not a regular file"));
    }

    if inode.check_permission(Permission::MAY_EXEC).is_err() {
        return Err(errno!(EACCES, "the inode is not executable"));
    }

    Ok(())
}
