use std::{
    env,
    io::{self, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    fs,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use crossterm::{cursor, execute, terminal, terminal::ClearType};

const FPS: u64 = 30;
const PIXELS: &[u8] = b" .:-=+*#%@";
const WORDS_PATH: &str = "text/text.txt";

fn main() -> Result<()> {
    let video_path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("video/Bad Apple.mp4"));

    if !video_path.exists() {
        bail!("видео не найдено: {}", video_path.display());
    }
    let words = load_words()?;

    let (columns, rows) = terminal::size().context("не удалось определить размер консоли")?;
    let width = columns.max(1) as usize / 2;
    let height = rows.saturating_sub(1).max(1) as usize;
    let frame_size = width * height;

    let mut ffmpeg = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            video_path.to_str().context("некорректный путь к видео")?,
            "-vf",
            &format!("scale={width}:{height},format=gray"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .context("не найден ffmpeg. Установите FFmpeg и добавьте его в PATH")?;

    let mut video = ffmpeg.stdout.take().context("не удалось получить видеопоток")?;
    let mut stdout = io::stdout();
    terminal::enable_raw_mode().context("не удалось включить режим консоли")?;
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;

    let result = play(&mut stdout, &mut video, frame_size, width, height, &words);

    execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    let status = ffmpeg.wait()?;
    result?;

    if !status.success() {
        bail!("ffmpeg не смог декодировать видео");
    }
    Ok(())
}

fn play(
    stdout: &mut io::Stdout,
    video: &mut impl Read,
    frame_size: usize,
    width: usize,
    height: usize,
    words: &[String],
) -> Result<()> {
    let mut frame = vec![0; frame_size];
    let frame_delay = Duration::from_millis(1000 / FPS);
    let mut frame_number = 0usize;

    loop {
        let mut received = 0;
        while received < frame_size {
            let count = video.read(&mut frame[received..])?;
            if count == 0 {
                return Ok(());
            }
            received += count;
        }

        execute!(stdout, cursor::MoveTo(0, 0))?;
        for row in frame.chunks_exact(width).take(height) {
            execute!(stdout, terminal::Clear(ClearType::CurrentLine))?;
            write_row(stdout, row, words, frame_number)?;
            writeln!(stdout)?;
        }
        stdout.flush()?;
        std::thread::sleep(frame_delay);
        frame_number += 1;
    }
}

fn load_words() -> Result<Vec<String>> {
    let text = fs::read_to_string(WORDS_PATH)
        .with_context(|| format!("не удалось прочитать список слов: {WORDS_PATH}"))?;
    let words = text
        .lines()
        .map(str::trim)
        .filter(|word| (word.len() == 2 || word.len() == 3) && word.bytes().all(|b| b.is_ascii_alphabetic()))
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();

    if words.is_empty() {
        bail!("в файле {WORDS_PATH} нет подходящих слов из 2 или 3 букв");
    }
    Ok(words)
}

fn write_row(
    stdout: &mut io::Stdout,
    row: &[u8],
    words: &[String],
    frame_number: usize,
) -> Result<()> {
    let mut position = 0;
    let mut previous_was_word = false;
    while position < row.len() {
        let brightness = row[position];
        let index = brightness as usize * (PIXELS.len() - 1) / 255;
        let symbol = PIXELS[index] as char;

        if (symbol == '%' || symbol == '@') && position + 1 < row.len() {
            let mut run_length = 1;
            while run_length < 3 && position + run_length < row.len() {
                let next_index = row[position + run_length] as usize * (PIXELS.len() - 1) / 255;
                let next_symbol = PIXELS[next_index] as char;
                if next_symbol != symbol {
                    break;
                }
                run_length += 1;
            }

            if run_length == 2 || run_length == 3 {
                let candidates = words.iter().filter(|word| word.len() == run_length).collect::<Vec<_>>();
                if !candidates.is_empty() {
                    let word = candidates[(frame_number + position) % candidates.len()];
                    if previous_was_word {
                        write!(stdout, "+")?;
                    }
                    write!(stdout, "{word}")?;
                    previous_was_word = true;
                    position += run_length;
                    continue;
                }
            }
        }

        write!(stdout, "{symbol}{symbol}")?;
        previous_was_word = false;
        position += 1;
    }
    Ok(())
}
