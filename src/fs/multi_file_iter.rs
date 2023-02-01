use crate::transcoding::metainfo::File;

#[derive(Debug)]
pub struct FileWithBounds<'a> {
    pub file: &'a File,
    pub start_within_iter_bounds: usize,
    pub end_within_iter_bounds: usize,
    pub file_bytes_before_iter: usize,
    pub file_bytes_after_iter: usize,
}

pub struct FilesInBounds<'a> {
    file_iter: Box<dyn Iterator<Item = &'a File> + 'a>,
    start_byte: usize,
    end_byte: usize,
    cur_idx: usize,
}

impl<'a> FilesInBounds<'a> {
    pub fn new(files: &'a [File], start_byte: usize, end_byte: usize) -> Self {
        let mut file_iter = files.iter().peekable();
        let mut file_start = 0;

        loop {
            let Some(file) = file_iter.peek() else {
                break;
            };

            let file_end = file_start + file.length as usize;

            if file_start < end_byte
                && (file_end > start_byte || (file.length == 0 && file_end >= start_byte))
            {
                break;
            }

            let _ = file_iter.next();
            file_start = file_end;
        }

        Self {
            file_iter: Box::new(file_iter),
            start_byte,
            end_byte,
            cur_idx: file_start,
        }
    }
}

impl<'a> Iterator for FilesInBounds<'a> {
    type Item = FileWithBounds<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let file = self.file_iter.next()?;

        let file_start = self.cur_idx;
        let file_end = file_start + file.length as usize;

        if file_start >= self.end_byte
            || (file_end <= self.start_byte && file.length > 0)
            || (file_end < self.start_byte && file.length == 0)
        {
            return None;
        }

        self.cur_idx = file_end;

        let start_of_file_after_self_start = std::cmp::max(self.start_byte, file_start);
        let end_of_file_before_self_end = std::cmp::min(self.end_byte, file_end);

        Some(FileWithBounds {
            file,
            start_within_iter_bounds: start_of_file_after_self_start - self.start_byte,
            end_within_iter_bounds: end_of_file_before_self_end - self.start_byte,
            file_bytes_before_iter: start_of_file_after_self_start - file_start,
            file_bytes_after_iter: file_end - end_of_file_before_self_end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::FilesInBounds;
    use crate::transcoding::metainfo::{File, Mode, Torrent};
    use tracing_test::traced_test;

    const ASIMOV_COLLECTION: &str = "resources/Isaac_Asimov_Collection.torrent";
    const ROBOTS_OF_DAWN_19_START: usize = 1_487_340_465;
    const ROBBIE_START: usize = 1_562_727_907; // brings me tears, asshole parents..

    fn get_files() -> Vec<File> {
        let torrent = Torrent::from_file(ASIMOV_COLLECTION).unwrap();

        let Mode::Multi { files } = torrent.info.mode else {
            panic!("expected multi mode");
        };

        files
    }

    #[traced_test]
    #[test]
    fn seems_alright() {
        let files = get_files();

        let robots_of_dawn_18 = FilesInBounds::new(
            &files,
            ROBOTS_OF_DAWN_19_START - 1,
            ROBBIE_START + (1 << 10),
        )
        .next()
        .unwrap();
        assert_eq!(
            "Robots of Dawn 18.mp3",
            robots_of_dawn_18.file.path.last().unwrap()
        );

        let robots_of_dawn_19 =
            FilesInBounds::new(&files, ROBOTS_OF_DAWN_19_START, ROBBIE_START + (1 << 10))
                .next()
                .unwrap();
        assert_eq!(
            "Robots of Dawn 19.mp3",
            robots_of_dawn_19.file.path.last().unwrap()
        );

        assert_eq!(
            3,
            FilesInBounds::new(&files, ROBOTS_OF_DAWN_19_START, ROBBIE_START + (1 << 10)).count()
        );

        assert_eq!(
            1055 - 20,
            FilesInBounds::new(&files, ROBOTS_OF_DAWN_19_START, usize::MAX).count()
        );

        assert_eq!(1055, FilesInBounds::new(&files, 0, usize::MAX).count());
    }
}
