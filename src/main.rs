use std::{
    env,
    fmt::Write as FmtWrite,
    io::{self, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    fs,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use crossterm::{cursor, execute, terminal};

const FPS: u64 = 30;
const WORDS_PATH: &str = "text/text.txt";
const CELL_WIDTH: usize = 4;

fn main() -> Result<()> {
    let video_path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("video/video.mp4"));

    if !video_path.exists() {
        bail!("видео не найдено: {}", video_path.display());
    }
    let words = load_words()?;

    let (source_width, source_height) = video_size(&video_path)?;
    let (columns, rows) = terminal::size().context("не удалось определить размер консоли")?;
    let (width, height) = fit_video(source_width, source_height, columns as usize, rows as usize);
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

    loop {
        let mut received = 0;
        while received < frame_size {
            let count = video.read(&mut frame[received..])?;
            if count == 0 {
                return Ok(());
            }
            received += count;
        }

        let (terminal_width, terminal_height) = terminal::size()
            .map(|size| (size.0 as usize, size.1 as usize))
            .unwrap_or((width * CELL_WIDTH, height + 1));
        let frame_width = width * CELL_WIDTH;
        let left = terminal_width.saturating_sub(frame_width) / 2;
        let top = terminal_height.saturating_sub(height) / 2;
        let mut output = String::new();
        for (row_number, row) in frame.chunks_exact(width).take(height).enumerate() {
            // Position every row explicitly. Newline would reset the cursor
            // to column one and break centering for narrow videos.
            output.push_str(&format!("\x1b[{};{}H\x1b[2K", top + row_number + 1, left + 1));
            write_row(&mut output, row, words, row_number)?;
        }
        output.push_str("\x1b[0m");
        write!(stdout, "{output}")?;
        stdout.flush()?;
        std::thread::sleep(frame_delay);
    }
}

fn video_size(video_path: &PathBuf) -> Result<(usize, usize)> {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error", "-select_streams", "v:0", "-show_entries",
            "stream=width,height", "-of", "csv=s=x:p=0",
            video_path.to_str().context("некорректный путь к видео")?,
        ])
        .output()
        .context("не найден ffprobe. Он устанавливается вместе с FFmpeg")?;

    let dimensions = String::from_utf8_lossy(&output.stdout);
    let (width, height) = dimensions
        .trim()
        .split_once('x')
        .context("ffprobe не вернул размеры видео")?;
    Ok((width.parse()?, height.parse()?))
}

fn fit_video(source_width: usize, source_height: usize, columns: usize, rows: usize) -> (usize, usize) {
    // One logical video pixel occupies two console columns, so compensate for
    // the terminal character cell being taller than it is wide.
    let available_width = columns / CELL_WIDTH;
    let available_height = rows.saturating_sub(1);
    let scale = (available_width as f64 / source_width as f64)
        .min(available_height as f64 / source_height as f64)
        .max(0.01);

    (
        ((source_width as f64 * scale).floor() as usize).max(1),
        ((source_height as f64 * scale).floor() as usize).max(1),
    )
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
    output: &mut String,
    row: &[u8],
    words: &[String],
    row_number: usize,
) -> Result<()> {
    let mut position = 0;
    let mut previous_was_word = false;
    while position < row.len() {
        let brightness = row[position] as usize;
        let render_word = brightness > 12
            && brightness > BAYER_4X4[row_number % 4][position % 4] as usize;

        if render_word {
            let word_index = brightness * words.len() / 256;
            let word = &words[word_index.min(words.len() - 1)];
            write_word(output, word, previous_was_word)?;
        } else {
            output.push_str("    ");
        }
        previous_was_word = render_word;
        position += 1;
    }
    Ok(())
}

const BAYER_4X4: [[u8; 4]; 4] = [
    [8, 136, 40, 168],
    [200, 72, 232, 104],
    [56, 184, 24, 152],
    [248, 120, 216, 88],
];

fn write_word(output: &mut String, word: &str, separator: bool) -> Result<()> {
    // A fixed-width cell preserves the image proportions while + separates
    // adjacent words without adding columns to the frame.
    output.push(if separator { '+' } else { ' ' });
    write!(output, "{word}")?;
    output.push_str(&" ".repeat(CELL_WIDTH - 1 - word.len()));
    Ok(())
}
