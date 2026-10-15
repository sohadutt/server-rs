use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
};

use env_logger::{Builder, Target};

use crate::config::{Config, LogLevel};

struct TeeWriter {
    console: bool,
    file: Option<File>,
}

impl Write for TeeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.console {
            io::stderr().write_all(bytes)?;
        }
        if let Some(file) = &mut self.file {
            file.write_all(bytes)?;
            file.flush()?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.console {
            io::stderr().flush()?;
        }
        if let Some(file) = &mut self.file {
            file.flush()?;
        }
        Ok(())
    }
}

pub fn initialize(config: &Config) -> io::Result<()> {
    let file = if config.log_file {
        fs::create_dir_all(&config.log_directory)?;
        Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(config.log_directory.join("server.log"))?,
        )
    } else {
        None
    };
    let mut builder = Builder::new();
    builder.filter_level(match config.log_level {
        LogLevel::Debug => log::LevelFilter::Debug,
        LogLevel::Info => log::LevelFilter::Info,
        LogLevel::Warn => log::LevelFilter::Warn,
        LogLevel::Error => log::LevelFilter::Error,
    });
    builder.format(|buffer, record| {
        writeln!(
            buffer,
            "{} [{}] {}",
            buffer.timestamp(),
            record.level(),
            record.args()
        )
    });
    builder.target(Target::Pipe(Box::new(TeeWriter {
        console: config.log_console || config.log_level == LogLevel::Debug,
        file,
    })));
    builder
        .try_init()
        .map_err(|error| io::Error::other(error.to_string()))
}
