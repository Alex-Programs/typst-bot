use std::collections::HashMap;
use std::str::FromStr;

use poise::async_trait;
use poise::serenity_prelude::{AttachmentType, GatewayIntents};
use tokio::sync::Mutex;

use crate::worker::Worker;
use crate::SOURCE_URL;

/// Prevent garbled output from codeblocks unwittingly terminated by their own content.
///
/// U+200C is a zero-width non-joiner.
/// It prevents the triple backtick from being interpreted as a codeblock but retains ligature support.
fn sanitize_code_block(raw: &str) -> String {
	raw.replace("```", "``\u{200c}`")
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid theme")]
struct InvalidTheme;

impl FromStr for Theme {
	type Err = InvalidTheme;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(match s {
			"transparent" => Self::Transparent,
			"light" => Self::Light,
			"dark" => Self::Dark,
			_ => return Err(InvalidTheme),
		})
	}
}

#[derive(Default, Debug, Clone, Copy)]
enum Theme {
	Transparent,
	Light,
	#[default]
	Dark,
}

impl Theme {
	const fn preamble(self) -> &'static str {
		match self {
			Self::Transparent => "",
			Self::Light => "#set page(fill: white)\n",
			Self::Dark => concat!(
				"#set page(fill: rgb(49, 51, 56))\n",
				"#set text(fill: rgb(219, 222, 225))\n",
			),
		}
	}
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid page size")]
struct InvalidPageSize;

impl FromStr for PageSize {
	type Err = InvalidPageSize;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(match s {
			"auto" => Self::Auto,
			"default" => Self::Default,
			_ => return Err(InvalidPageSize),
		})
	}
}

#[derive(Default, Debug, Clone, Copy)]
enum PageSize {
	#[default]
	Auto,
	Default,
}

impl PageSize {
	const fn preamble(self) -> &'static str {
		match self {
			Self::Auto => "#set page(width: auto, height: auto, margin: 10pt)\n",
			Self::Default => "",
		}
	}
}

#[derive(Default, Debug, Clone, Copy)]
struct Preamble {
	page_size: PageSize,
	theme: Theme,
}

impl Preamble {
	fn preamble(self) -> String {
		let page_size = self.page_size.preamble();
		let theme = self.theme.preamble();
		if theme.is_empty() && page_size.is_empty() {
			String::new()
		} else {
			format!(
				concat!(
					"// Begin preamble\n",
					"// Page size:\n",
					"{page_size}",
					"// Theme:\n",
					"{theme}",
					"// End preamble\n",
				),
				page_size = page_size,
				theme = theme,
			)
		}
	}
}

struct Data {
	pool: Mutex<Worker>,
}

type PoiseError = Box<dyn std::error::Error + Send + Sync + 'static>;
type Context<'a> = poise::Context<'a, Data, PoiseError>;

#[derive(Debug, Default)]
struct RenderFlags {
	preamble: Preamble,
}

#[async_trait]
impl<'a> poise::PopArgument<'a> for RenderFlags {
	async fn pop_from(
		args: &'a str,
		attachment_index: usize,
		ctx: &serenity::prelude::Context,
		message: &poise::serenity_prelude::Message,
	) -> Result<(&'a str, usize, Self), (PoiseError, Option<String>)> {
		fn inner(raw: &HashMap<String, String>) -> Result<RenderFlags, PoiseError> {
			let mut parsed = RenderFlags::default();

			for (key, value) in raw {
				match key.as_str() {
					"theme" => {
						parsed.preamble.theme = value.parse().map_err(|_| "invalid theme")?;
					}
					"pagesize" => {
						parsed.preamble.page_size = value.parse().map_err(|_| "invalid page size")?;
					}
					_ => {
						return Err(format!("unrecognized flag {key:?}").into());
					}
				}
			}

			Ok(parsed)
		}

		let (remaining, pos, raw) =
			poise::prefix_argument::KeyValueArgs::pop_from(args, attachment_index, ctx, message).await?;

		inner(&raw.0)
			.map(|parsed| (remaining, pos, parsed))
			.map_err(|error| (error, None))
	}
}

fn render_help() -> String {
	format!(
		"See https://discord.com/channels/759332813446316042/1026220421550460968/1135151260245430435"
	)
}

#[poise::command(
	prefix_command,
	track_edits,
	broadcast_typing,
	user_cooldown = 1,
	help_text_fn = "render_help"
)]
async fn render(
	ctx: Context<'_>,
	#[description = "Flags"] flags: RenderFlags,
	#[description = "Code to render"] code: poise::prefix_argument::CodeBlock,
) -> Result<(), PoiseError> {
	let pool = &ctx.data().pool;

	let mut source = code.code;
	source.insert_str(0, &flags.preamble.preamble());

	let res = pool.lock().await.render(source).await;

	match res {
		Ok(res) => {
			ctx
				.send(|reply| {
					reply
						.attachment(AttachmentType::Bytes {
							data: res.image.into(),
							filename: "rendered.png".into(),
						})
						.reply(true);

					if let Some(more_pages) = res.more_pages {
						let more_pages = more_pages.get();
						reply.content(format!(
							"Note: {more_pages} more page{s} ignored",
							s = if more_pages == 1 { "" } else { "s" }
						));
					}

					reply
				})
				.await?;
		}
		Err(error) => {
			let error = sanitize_code_block(&format!("{error:?}"));
			ctx
				.send(|reply| {
					reply
						.content(format!("An error occurred:\n```\n{error}```"))
						.reply(true)
				})
				.await?;
		}
	}

	Ok(())
}

/// Show this menu
#[poise::command(prefix_command, track_edits, slash_command)]
async fn help(
	ctx: Context<'_>,
	#[description = "Specific command to show help about"] command: Option<String>,
) -> Result<(), PoiseError> {
	let config = poise::builtins::HelpConfiguration {
		extra_text_at_bottom: "\
Type ?help command for more info on a command.
You can edit your message to the bot and the bot will edit its response.
Commands prefixed with / can be used as slash commands and prefix commands.
Commands prefixed with ? can only be used as prefix commands.",
		ephemeral: true,
		..Default::default()
	};
	poise::builtins::help(ctx, command.as_deref(), config).await?;
	Ok(())
}

/// Get a link to the bot's source.
#[poise::command(prefix_command, slash_command)]
async fn source(ctx: Context<'_>) -> Result<(), PoiseError> {
	ctx
		.send(|reply| reply.content(format!("<{SOURCE_URL}>")).reply(true))
		.await?;

	Ok(())
}

/// Get the AST for the given code.
///
/// Syntax: `?ast <code block>`
///
/// **Examples**
///
/// ```
/// ?ast `hello, world!`
///
/// ?ast ``‌`
/// = Heading!
///
/// And some text.
///
/// #lorem(100)
/// ``‌`
/// ```
#[poise::command(prefix_command, track_edits, broadcast_typing)]
async fn ast(
	ctx: Context<'_>,
	#[description = "Code to parse"] code: poise::prefix_argument::CodeBlock,
) -> Result<(), PoiseError> {
	let pool = &ctx.data().pool;

	let res = pool.lock().await.ast(code.code).await;

	match res {
		Ok(ast) => {
			let ast = sanitize_code_block(&ast);
			let ast = format!("```{ast}```");

			ctx.send(|reply| reply.content(ast).reply(true)).await?;
		}
		Err(error) => {
			ctx
				.send(|reply| {
					reply
						.content(format!("An error occurred:\n```\n{error}```"))
						.reply(true)
				})
				.await?;
		}
	}

	Ok(())
}

#[poise::command(prefix_command, track_edits)]
async fn version(ctx: Context<'_>) -> Result<(), PoiseError> {
	let pool = &ctx.data().pool;

	let res = pool.lock().await.version().await;

	match res {
		Ok(version) => {
			let content = format!(
				"The bot is using Typst version {}, git hash {}",
				version.version, version.git_hash,
			);
			ctx.send(|reply| reply.content(content).reply(true)).await?;
		}
		Err(error) => {
			ctx
				.send(|reply| {
					reply
						.content(format!("An error occurred:\n```\n{error}```"))
						.reply(true)
				})
				.await?;
		}
	}

	Ok(())
}

pub async fn run() {
	let pool = Worker::spawn().unwrap();

	let edit_tracker_time = std::time::Duration::from_secs(3600);

	let framework = poise::Framework::builder()
		.options(poise::FrameworkOptions {
			prefix_options: poise::PrefixFrameworkOptions {
				prefix: Some("?".to_owned()),
				edit_tracker: Some(poise::EditTracker::for_timespan(edit_tracker_time)),
				..Default::default()
			},
			commands: vec![render(), help(), source(), ast(), version()],
			..Default::default()
		})
		.token(std::env::var("DISCORD_TOKEN").expect("need `DISCORD_TOKEN` env var"))
		.intents(GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT)
		.setup(|ctx, _ready, framework| {
			Box::pin(async move {
				poise::builtins::register_globally(ctx, &framework.options().commands).await?;
				Ok(Data {
					pool: Mutex::new(pool),
				})
			})
		});

	eprintln!("ready");

	framework.run().await.unwrap();
}
