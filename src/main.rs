use anyhow::{anyhow, Result};
use regex::Regex;
use std::fs::File;
use std::io::BufReader;
use std::io::Write;
use std::process::Command;
use std::process::Stdio;

#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate derive_builder;

use strum_macros::EnumString;
#[derive(EnumString, Serialize, Deserialize, Debug, PartialEq, Clone)]
enum TagStatus {
    Normal,
    Missing,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct CommentTag {
    revision: String,
    line: i32,
    status: TagStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Comment {
    text: String,
    tags: Vec<CommentTag>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FileData {
    path: String,
    comments: Vec<Comment>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RootData {
    files: Vec<FileData>,
}

#[derive(Builder, Debug, PartialEq, Clone)]
struct GitBlameOption {
    #[builder(setter(into))]
    file: String,
    #[builder(setter(into, strip_option))]
    repo_path: Option<String>,
    #[builder(default = "false")]
    reverse: bool,
    #[builder(default = "false")]
    line_number: bool,
    #[builder(setter(into))]
    revision: String,
}

impl GitBlameOption {
    fn build_command(&self) -> Command {
        let mut command = Command::new("git");
        if let Some(repo_path) = &self.repo_path {
            command.arg("-C").arg(repo_path);
        }
        command.arg("blame");
        if self.reverse {
            command.arg("--reverse");
        }
        if self.line_number {
            command.arg("-n");
        }
        command.arg(&self.revision).arg(&self.file);
        command
    }
}
fn git_current_revision(repo_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let child = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("rev-parse")
        .arg("--short")
        .arg("HEAD")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git command failed to start");
    let output = child.wait_with_output()?;
    Ok(String::from_utf8(output.stdout).unwrap())
}

fn git_merge_base_is_ancestor(
    repo_path: &str,
    revision1: &str,
    revision2: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let child = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("merge-base")
        .arg("--is-ancestor")
        .arg(revision1)
        .arg(revision2)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git command failed to start");
    let output = child.wait_with_output()?;
    Ok(output.status.code().unwrap() == 0)
}

fn git_blame_reverse(git_blame_option: &GitBlameOption) -> Result<Vec<GitBlameResult>> {
    let child = git_blame_option
        .build_command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git command failed to start");

    let mut results: Vec<GitBlameResult> = Vec::new();
    let output = child.wait_with_output()?;
    let exit_code = output.status.code().unwrap();
    if exit_code != 0 {
        let stderr_output = String::from_utf8(output.stderr).unwrap();
        return Err(anyhow!(
            "Failed to run git command: exit_code={}, stderr={}",
            exit_code,
            stderr_output
        ));
    }
    let stdout_output = String::from_utf8(output.stdout).unwrap();
    for line in stdout_output.lines() {
        // TODO: add test code
        let result = GitBlameResult::new_from_line(
            r"^(?P<revision>[^ ]+) (?P<new_line>[^ ]+) [^)]+ (?P<orig_line>[0-9]+)\)",
            line,
        );
        results.push(result);
    }
    Ok(results)
}

#[derive(Debug, PartialEq)]
struct GitBlameResult {
    revision: String,
    orig_line_number: i32,
    new_line_number: i32,
}
impl GitBlameResult {
    fn new_from_line(pattern: &str, line: &str) -> GitBlameResult {
        let re = Regex::new(pattern).unwrap();
        let captures = re.captures(line).unwrap();

        let revision = captures.name("revision").unwrap().as_str();
        let new_line_number = captures
            .name("new_line")
            .unwrap()
            .as_str()
            .parse::<i32>()
            .unwrap();
        let orig_line_number = captures
            .name("orig_line")
            .unwrap()
            .as_str()
            .parse::<i32>()
            .unwrap();
        return GitBlameResult {
            new_line_number: new_line_number,
            orig_line_number: orig_line_number,
            revision: revision.to_string(),
        };
    }
}

#[allow(dead_code)]
fn get_sample_data() -> RootData {
    let data = RootData {
        files: vec![FileData {
            path: String::from("./README.md"),
            comments: vec![
                Comment {
                    text: String::from("hello A"),
                    tags: vec![CommentTag {
                        revision: String::from("39690ed"),
                        line: 1,
                        status: TagStatus::Normal,
                    }],
                },
                Comment {
                    text: String::from("hello B"),
                    tags: vec![CommentTag {
                        revision: String::from("39690ed"),
                        line: 2,
                        status: TagStatus::Normal,
                    }],
                },
            ],
        }],
    };
    data
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_repo = "./example-repo";
    let file = File::open("in.json").unwrap();
    let reader = BufReader::new(file);
    let data: RootData = serde_json::from_reader(reader).unwrap();

    let mut new_data = data.clone();
    let current_revision = git_current_revision(target_repo).unwrap();
    for (file_index, file_data) in data.files.iter().enumerate() {
        println!("file path = {}", file_data.path);
        for (comment_index, comment) in file_data.comments.iter().enumerate() {
            println!("comment text = {}", comment.text);
            for tag in &comment.tags {
                println!("comment text = {:?}", tag);
                if tag.revision == current_revision {
                    println!("same revision skip: {:?}", tag);
                    continue;
                }
                let is_ancestor =
                    git_merge_base_is_ancestor(target_repo, &tag.revision, "HEAD").unwrap();
                println!("is_ancestor = {:?}", is_ancestor);
                if is_ancestor {
                    // if use -L option, there is no need to parse entire file lines
                    // if no use -L option and cache the result, we can reuse the result
                    let git_blame_option = GitBlameOptionBuilder::default()
                        .file(&file_data.path)
                        .repo_path(target_repo)
                        .reverse(true)
                        .line_number(true)
                        .revision(tag.revision.clone() + "..HEAD")
                        .build()
                        .unwrap();
                    let results = git_blame_reverse(&git_blame_option).unwrap();
                    if 1 <= tag.line && tag.line <= results.len() as i32 {
                        let new_info = &results[(tag.line - 1) as usize];
                        // for debug
                        // println!(
                        //     "new revision:{}, new line number:{}",
                        //     new_info.revision, new_info.new_line_number
                        // );
                        new_data.files[file_index].comments[comment_index]
                            .tags
                            .push(CommentTag {
                                revision: String::from(&new_info.revision),
                                line: new_info.new_line_number,
                                status: TagStatus::Normal,
                            });
                    }
                    for result in results {
                        println!("{:?}", result);
                    }
                }
            }
        }
    }

    let serialized = serde_json::to_string_pretty(&data)?;
    println!("[input data]");
    println!("{}", serialized);

    let new_serialized = serde_json::to_string_pretty(&new_data)?;
    println!("[output data]");
    println!("{}", new_serialized);

    let mut outfile = File::create("out.json")?;
    outfile.write_all(new_serialized.as_bytes())?;
    Ok(())
}
