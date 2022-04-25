#![allow(unused)]

mod connection;
mod error;
mod logger;
mod msg;
mod socket;
mod stdio;

use bytes::Buf;
use bytes::BytesMut;
use connection::Connection;
use crossbeam_channel::SendError;
use emacs::{defun, Env, IntoLisp, Result, Value};
use logger::Logger;
use msg::Message;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::File;
use std::result::Result as RustResult;
use std::str::Utf8Error;
use stdio::IoThreads;

use std::{
    collections::{HashMap, VecDeque},
    fmt::{format, Debug},
    io::{Read, Write},
    mem::MaybeUninit,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex, Once},
    thread::{self, JoinHandle, Thread},
};

#[derive(Debug)]
struct FileInfo {
    pub uri: String, // 文件名
}

struct LspServer {
    pub child: Option<Child>,
    pub nick_name: String,
    pub initialized: bool,               // 是否已经启动完
    pub capabilities: String,            // TODO 类型
    pub usable_capabilites: String,      // TODO
    pub uris: HashMap<String, FileInfo>, // key: uri
    transport: Option<Connection>,       // TODO StdioConnection替换为模板参数
    threads: Option<IoThreads>,
}

impl LspServer {
    pub fn new(
        root_uri: String,
        cmd: String,
        cmd_args: String,
        lsp_args: String,
    ) -> Option<LspServer> {
        let args = cmd_args.split_ascii_whitespace().collect::<Vec<&str>>();

        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn();

        if let Ok(mut c) = child {
            let mut server = LspServer {
                child: None,
                nick_name: "".to_string(),
                initialized: false,
                capabilities: "".to_string(),
                usable_capabilites: "".to_string(),
                uris: HashMap::new(),
                transport: None,
                threads: None,
            };

            let mut stdin = c.stdin.take().unwrap();
            let mut stdout = c.stdout.take().unwrap();

            let (mut transport, mut threads) = Connection::stdio(stdin, stdout);

            server.transport = Some(transport);
            server.threads = Some(threads);

            Some(server)
        } else {
            None
        }
    }

    pub fn request(&mut self, request: String) -> Option<String> {
        if self.transport.is_some() {
            let msg: Message = serde_json::from_str(&request).unwrap();

            let result = self.transport.as_ref().unwrap().write(msg);
        }

        None
    }

    pub fn notify(&mut self, notification: String) {}
}

struct Project {
    pub root_uri: String,                    // 项目根目录
    pub servers: HashMap<String, LspServer>, // 每个LspServer处理一种类型的文件
    pub client_capabilities: String,         // TODO 类型
}

impl Project {
    pub fn new(root_uri: String, client_capabilities: String) -> Project {
        Project {
            root_uri,
            servers: HashMap::new(),
            client_capabilities,
        }
    }
}

// Emacs won't load the module without this.
emacs::plugin_is_GPL_compatible!();

// Register the initialization hook that Emacs will call when it loads the module.
#[emacs::module(name("lspce-module"))]
fn init(env: &Env) -> Result<Value<'_>> {
    Logger::log("Done loading!");

    env.message("Done loading!")
}

fn projects() -> &'static Arc<Mutex<HashMap<String, Project>>> {
    static mut PROJECTS: MaybeUninit<Arc<Mutex<HashMap<String, Project>>>> = MaybeUninit::uninit();
    static ONCE: Once = Once::new();

    ONCE.call_once(|| unsafe {
        PROJECTS
            .as_mut_ptr()
            .write(Arc::new(Mutex::new(HashMap::new())))
    });

    unsafe { &*PROJECTS.as_mut_ptr() }
}

#[defun]
fn connect(
    env: &Env,
    root_uri: String,
    file_type: String,
    cmd: String,
    cmd_args: String,
    lsp_args: String,
) -> Result<String> {
    Logger::log(&format!(
        "start initializing server for file_type {} in project {}",
        file_type, root_uri
    ));
    if (server_running(env, root_uri.clone(), file_type.clone()).unwrap()) {
        Logger::log(&format!(
            "server created already for file_type {} in project {}",
            file_type, root_uri
        ));
        return Ok("server created already, return now.".to_string());
    }

    let mut server = LspServer::new(
        root_uri.clone(),
        cmd.clone(),
        cmd_args.clone(),
        lsp_args.clone(),
    );
    if let Some(mut s) = server {
        let mut projects = projects().lock().unwrap();
        let mut project = projects.get_mut(&root_uri);
        if let Some(p) = project.as_mut() {
            initialize(env, root_uri.clone(), &mut s, lsp_args);

            p.servers.insert(file_type, s);
        } else {
            let mut proj = Project::new(root_uri.clone(), "".to_string());
            initialize(env, root_uri.clone(), &mut s, lsp_args);

            proj.servers.insert(file_type, s);

            projects.insert(root_uri.clone(), proj);
        }

        Logger::log(&format!("connected to server."));
    } else {
        Logger::log(&format!("connect failed"));
    }

    Ok("server created".to_string())
}

fn initialize(
    env: &Env,
    root_uri: String,
    server: &mut LspServer,
    lsp_args: String,
) -> Result<String> {
    Logger::log(&format!("initialize request {:#?}", lsp_args));

    let msg: Message = serde_json::from_str(&lsp_args).unwrap();

    let write_result = server.transport.as_mut().unwrap().write(msg);
    match write_result {
        Ok(_) => loop {
            thread::sleep(std::time::Duration::from_millis(100));

            let resp = server.transport.as_mut().unwrap().read();
            Logger::log(&format!("initialize ok {:#?}", resp));
            if resp.is_some() {
                break;
                // TODO
            } else {
                // TODO
            }
        },
        Err(error) => {
            Logger::log(&format!("initialize error {:#?}", error));
        }
    }

    Ok("initialized".to_string())
}

// 返回server信息 TODO
#[defun]
fn server(env: &Env, root_uri: String, file_type: String) -> Result<bool> {
    let projects = projects().lock().unwrap();

    Ok(false)
}

#[defun]
fn server_running(env: &Env, root_uri: String, file_type: String) -> Result<bool> {
    let projects = projects().lock().unwrap();

    if let Some(p) = projects.get(&root_uri) {
        if let Some(s) = p.servers.get(&file_type) {
            return Ok(true);
        }
    }

    Ok(false)
}

fn _server(root_uri: String, file_type: String) -> Option<&'static LspServer> {
    let projects = projects().lock().unwrap();

    None
}

#[defun]
fn request(env: &Env, root_uri: String, file_type: String, req: String) -> Result<String> {
    let projects = projects().lock().unwrap();

    Ok("".to_string())
}

#[defun]
fn notify(env: &Env, root_uri: String, file_type: String, req: String) -> Result<()> {
    let projects = projects().lock().unwrap();

    Ok(())
}
