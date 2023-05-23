#![allow(dead_code)]
use std::{fs::File, io::{Seek, Write}};

pub struct CleanFile {
	path: String,
	file: Option<File>,
	created: bool,
	written_to: bool,
}
impl CleanFile {
	pub fn open(path: String, create: bool) -> Result<Self, &'static str> {
		let mut file_options = File::options();
		file_options.read(true).write(true).create_new(create);
		
		let file = file_options.open(&(path)).map_err(|err| match err.kind() {
			std::io::ErrorKind::AlreadyExists => "file already exists",
			_ => "failed to open file"
		})?;

		return Ok(Self { path, file: Some(file), created: create, written_to: false, });
	}

	fn delete_file(&mut self) -> Result<(), &'static str> {
		return if let Some(_) = self.file.take() {
			std::fs::remove_file(&(self.path)).map_err(|_| "failed to delete file")
		} else { Ok(()) };
	}
	pub fn delete(mut self) -> Result<(), &'static str> {
		return self.delete_file();
	}

	pub fn write<T: AsRef<[u8]>>(&mut self, bytes: T) -> Result<(), &'static str> {
		let file: &mut File = self.file.as_mut().ok_or("file deleted")?;
		file.set_len(0).map_err(|_| "failed to truncate file")?;
		file.seek(std::io::SeekFrom::Start(0)).map_err(|_| "failed to seek file")?;
		file.write_all(bytes.as_ref()).map_err(|_| "failed to write to file")?;

		self.written_to = true;
		return Ok(());
	}

	pub fn file(&self) -> Result<&File, &'static str> {
		return self.file.as_ref().ok_or("file deleted");
	}

	pub fn path(&self) -> &str {
		return &(self.path);
	}
}
impl Drop for CleanFile {
    fn drop(&mut self) {
    	if self.created && !self.written_to {
    		self.delete_file().unwrap();
    	}
    }
}