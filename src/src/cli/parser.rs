use crate::error::AppError;

use super::args::{parse_node_id_text, set_once, CliArgs};
use super::content::ContentSource;

pub(super) struct ShowCommand {
    pub(super) node_id: i64,
    pub(super) trash_only: bool,
}

pub(super) struct CreateCommand {
    pub(super) parent_id: i64,
    pub(super) title: String,
    pub(super) content: ContentSource,
}

pub(super) struct EditCommand {
    pub(super) node_id: i64,
    pub(super) content: ContentSource,
    pub(super) append: bool,
}

pub(super) struct RenameCommand {
    pub(super) node_id: i64,
    pub(super) title: String,
}

pub(super) struct MoveCommand {
    pub(super) node_id: i64,
    pub(super) parent_id: i64,
}

pub(super) fn parse_show_args(mut args: CliArgs) -> Result<ShowCommand, AppError> {
    let mut trash_only = false;
    let mut node_id = None;

    while let Some(argument) = args.pop_optional_string()? {
        if argument == "--trash" {
            if trash_only {
                return Err(AppError::user("--trash 옵션은 한 번만 사용할 수 있습니다."));
            }
            trash_only = true;
            continue;
        }

        if node_id.is_some() {
            return Err(AppError::user(format!(
                "show 명령에서 알 수 없는 인수입니다: {argument}"
            )));
        }
        node_id = Some(parse_node_id_text(&argument, "node_id")?);
    }

    let node_id = node_id.ok_or_else(|| AppError::user("show 명령에는 <node_id>가 필요합니다."))?;
    Ok(ShowCommand {
        node_id,
        trash_only,
    })
}

pub(super) fn parse_create_args(mut args: CliArgs) -> Result<CreateCommand, AppError> {
    let mut parent_id = None;
    let mut title = None;
    let mut content = ContentSource::None;

    while let Some(option) = args.pop_optional_string()? {
        match option.as_str() {
            "--parent" => set_once(&mut parent_id, args.pop_node_id("--parent")?, "--parent")?,
            "--title" => set_once(&mut title, args.pop_string("--title")?, "--title")?,
            "--content" => {
                set_content_source(
                    &mut content,
                    ContentSource::Literal(args.pop_string("--content")?),
                )?;
            }
            "--content-file" => {
                set_content_source(
                    &mut content,
                    ContentSource::File(args.pop_path("--content-file")?),
                )?;
            }
            "--stdin" => set_content_source(&mut content, ContentSource::Stdin)?,
            _ => {
                return Err(AppError::user(format!(
                    "create 명령에서 알 수 없는 인수입니다: {option}"
                )));
            }
        }
    }

    let parent_id = parent_id
        .ok_or_else(|| AppError::user("create 명령에는 --parent <node_id> 옵션이 필요합니다."))?;
    let title = title
        .ok_or_else(|| AppError::user("create 명령에는 --title <title> 옵션이 필요합니다."))?;

    Ok(CreateCommand {
        parent_id,
        title,
        content,
    })
}

pub(super) fn parse_edit_args(mut args: CliArgs) -> Result<EditCommand, AppError> {
    let node_id = args.pop_node_id("node_id")?;
    let mut content = ContentSource::None;
    let mut append = false;

    while let Some(option) = args.pop_optional_string()? {
        match option.as_str() {
            "--content" => {
                set_content_source(
                    &mut content,
                    ContentSource::Literal(args.pop_string("--content")?),
                )?;
            }
            "--content-file" => {
                set_content_source(
                    &mut content,
                    ContentSource::File(args.pop_path("--content-file")?),
                )?;
            }
            "--stdin" => set_content_source(&mut content, ContentSource::Stdin)?,
            "--append" => {
                if append {
                    return Err(AppError::user(
                        "--append 옵션은 한 번만 사용할 수 있습니다.",
                    ));
                }
                append = true;
            }
            _ => {
                return Err(AppError::user(format!(
                    "edit 명령에서 알 수 없는 인수입니다: {option}"
                )));
            }
        }
    }

    if matches!(content, ContentSource::None) {
        return Err(AppError::user(
            "edit 명령에는 --content, --content-file, --stdin 중 하나가 필요합니다.",
        ));
    }

    Ok(EditCommand {
        node_id,
        content,
        append,
    })
}

pub(super) fn parse_rename_args(mut args: CliArgs) -> Result<RenameCommand, AppError> {
    let node_id = args.pop_node_id("node_id")?;
    let mut title = None;

    while let Some(option) = args.pop_optional_string()? {
        match option.as_str() {
            "--title" => set_once(&mut title, args.pop_string("--title")?, "--title")?,
            _ => {
                return Err(AppError::user(format!(
                    "rename 명령에서 알 수 없는 인수입니다: {option}"
                )));
            }
        }
    }

    let title = title
        .ok_or_else(|| AppError::user("rename 명령에는 --title <title> 옵션이 필요합니다."))?;
    Ok(RenameCommand { node_id, title })
}

pub(super) fn parse_search_args(mut args: CliArgs) -> Result<String, AppError> {
    let query = args.pop_string("query")?;
    args.ensure_empty("search")?;
    Ok(query)
}

pub(super) fn parse_node_id_args(mut args: CliArgs, command: &str) -> Result<i64, AppError> {
    let node_id = args.pop_node_id("node_id")?;
    args.ensure_empty(command)?;
    Ok(node_id)
}

pub(super) fn parse_move_args(mut args: CliArgs) -> Result<MoveCommand, AppError> {
    let node_id = args.pop_node_id("node_id")?;
    let mut parent_id = None;

    while let Some(option) = args.pop_optional_string()? {
        match option.as_str() {
            "--parent" => set_once(&mut parent_id, args.pop_node_id("--parent")?, "--parent")?,
            _ => {
                return Err(AppError::user(format!(
                    "move 명령에서 알 수 없는 인수입니다: {option}"
                )));
            }
        }
    }

    let parent_id = parent_id
        .ok_or_else(|| AppError::user("move 명령에는 --parent <node_id> 옵션이 필요합니다."))?;

    Ok(MoveCommand { node_id, parent_id })
}

fn set_content_source(target: &mut ContentSource, value: ContentSource) -> Result<(), AppError> {
    if !matches!(target, ContentSource::None) {
        return Err(AppError::user(
            "본문 입력 옵션은 --content, --content-file, --stdin 중 하나만 사용할 수 있습니다.",
        ));
    }

    *target = value;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_content_sources_are_rejected() {
        let mut source = ContentSource::None;
        set_content_source(&mut source, ContentSource::Literal("one".to_owned()))
            .expect("first content source should be accepted");

        assert!(set_content_source(&mut source, ContentSource::Stdin).is_err());
    }
}
