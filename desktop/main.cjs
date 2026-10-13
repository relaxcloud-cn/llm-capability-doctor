'use strict';
const {app,BrowserWindow,ipcMain,dialog}=require('electron');
const path=require('node:path');
const fs=require('node:fs/promises');
const {spawn,execFile}=require('node:child_process');
const readline=require('node:readline');
const crypto=require('node:crypto');
const createReports=require('./reports.cjs');
let mainWindow,worker,busy=false,quitting=false,root,workerReady=false,confirmingQuit=false;
const reports=createReports({BrowserWindow,resolveFile:id=>recordFile(id,'report.html'),isQuitting:()=>quitting,canAutoOpen:()=>Boolean(mainWindow&&!mainWindow.isDestroyed())&&!process.argv.includes('--agentcheck-no-open-report')&&!['SSH_CONNECTION','SSH_TTY','SSH_CLIENT','CI'].some(name=>process.env[name]!==undefined),onAutoError:()=>publish({type:'error',message:'报告已保存，但未能自动打开。请点击“查看报告”重试；检测结果不受影响。'})});
const pending=new Map(),idleWaiters=[];
let sequence=0,eventTape=[],uiReadyResolve;
const uiReady=new Promise(resolve=>uiReadyResolve=resolve);
const flag=name=>{const index=process.argv.indexOf(name);return index>=0?process.argv[index+1]:null;};
const cli=flag('--agentcheck-cli')||process.env.AGENTCHECK_CLI_PATH;
if(process.env.AGENTCHECK_SOFTWARE_RENDERING==='1')app.disableHardwareAcceleration();
function publish(message){
 message.sequence=++sequence;
 if(message.type==='event'){if(message.data?.event?.type==='started')eventTape=[];eventTape.push(message);if(eventTape.length>5000)eventTape.shift();}
 if(message.type==='state'){busy=Boolean(message.busy);if(!busy){idleWaiters.splice(0).forEach(resolve=>resolve());}}
 if(message.type==='event'&&message.data?.event?.type==='started')busy=true;
 if(mainWindow&&!mainWindow.isDestroyed())mainWindow.webContents.send('agentcheck:event',message);
 if(message.type==='event'&&message.data?.event?.type==='finished')void reports.finished(message.data.event.run);
}
function request(type,data={}){
 if(!worker||!workerReady)return Promise.reject(new Error('检测执行器尚未就绪'));
 const id=crypto.randomUUID();
 return new Promise((resolve,reject)=>{
  pending.set(id,{resolve,reject});worker.stdin.write(JSON.stringify({id,type,...data})+'\n',error=>{if(error){pending.delete(id);reject(new Error('无法向执行器发送操作'));}});
 });
}
function idle(){return !busy?Promise.resolve():new Promise(resolve=>idleWaiters.push(resolve));}
function waitBounded(promise,ms){let timer;return Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('执行器未在规定时间内停止')),ms);})]).finally(()=>clearTimeout(timer));}
async function killWorkerTree(){
 if(!worker?.pid)return;
 const pid=worker.pid;
 if(process.platform==='win32'){
  await new Promise(resolve=>execFile(path.join(process.env.SystemRoot||'C:\\Windows','System32','taskkill.exe'),['/PID',String(pid),'/T','/F'],()=>resolve()));
 }else{
  const tree=await new Promise(resolve=>execFile('/bin/ps',['-axo','pid=,ppid='],(error,stdout)=>resolve(error?'':stdout)));
  const rows=tree.trim().split('\n').map(line=>line.trim().split(/\s+/).map(Number));
  const children=new Set([pid]);let changed=true;
  while(changed){changed=false;for(const [id,parent] of rows){if(children.has(parent)&&!children.has(id)){children.add(id);changed=true;}}}
  for(const id of [...children].reverse()){try{process.kill(id,'SIGKILL');}catch{}}
 }
}
function configFromInput(input){
 if(!input||typeof input!=='object')throw new Error('请填写模型连接信息');
 const {base_url,model,api_key}=input;
 if(typeof base_url!=='string'||base_url.length>4096||typeof model!=='string'||model.length>256||typeof api_key!=='string'||api_key.length>8192)throw new Error('连接信息的格式或长度不正确');
 const protocol=input.protocol===null||input.protocol===undefined||input.protocol==='auto'?null:input.protocol;
 if(protocol!==null&&!['chat-completions','responses'].includes(protocol))throw new Error('接口协议不正确');
 return {base_url,model,api_key,protocol};
}
async function recordFile(id,name){
 if(typeof id!=='string'||!/^AC-[A-Za-z0-9_-]{1,125}$/.test(id))throw new Error('检测编号不正确');
 const file=await fs.realpath(path.join(root,id,name));const base=await fs.realpath(root);
 if(!file.startsWith(base+path.sep))throw new Error('文件不属于本机检测记录');
 return file;
}
async function openReport(id){return reports.open(id);}
async function requestQuit(){
 if(quitting||confirmingQuit)return;
 if(!busy){await stopAndQuit();return;}
 confirmingQuit=true;
 try{
  const options={type:'question',title:'AgentCheck',message:'停止检测并关闭？',detail:'已完成结果会保留，正在执行的任务将停止。',buttons:['继续检测','停止并关闭'],defaultId:0,cancelId:0};
  const {response}=await (mainWindow&&!mainWindow.isDestroyed()?dialog.showMessageBox(mainWindow,options):dialog.showMessageBox(options));
  if(response===1)await stopAndQuit();
 }finally{confirmingQuit=false;}
}
async function stopAndQuit(){
 if(quitting)return;quitting=true;
 if(busy){const stopped=idle();try{await request('cancel');await waitBounded(stopped,35000);}catch{await killWorkerTree();}}
 if(worker){worker.stdin.end();await waitBounded(new Promise(resolve=>{if(worker.exitCode!==null)resolve();else worker.once('exit',resolve);}),5000).catch(killWorkerTree);}
 app.quit();
}
async function startWorker(){
 if(!cli)throw new Error('请通过 AgentCheck 命令行入口启动桌面界面');
 root=flag('--agentcheck-report-dir')||path.join(app.getPath('userData'),'results');root=path.resolve(root);
 await fs.mkdir(root,{recursive:true});
 worker=spawn(cli,['--worker','--report-dir',root],{stdio:['pipe','pipe','pipe'],windowsHide:true});
 // Do not forward worker stderr or credentials to the renderer or app logs.
 worker.stderr.on('data',()=>{});
 const ready=new Promise((resolve,reject)=>{
  worker.once('error',()=>reject(new Error('无法启动检测执行器')));
  worker.once('exit',()=>{for(const p of pending.values())p.reject(new Error('检测执行器已退出'));pending.clear();workerReady=false;if(!quitting)publish({type:'error',message:'检测执行器已退出，已保存的结果仍可查看'});reject(new Error('检测执行器提前退出'));});
  const lines=readline.createInterface({input:worker.stdout});
  lines.on('line',line=>{
   if(line.length>16*1024*1024)return;
   let message;try{message=JSON.parse(line);}catch{return;}
   if(message.type==='ready'){workerReady=true;resolve();return;}
   if(message.type==='response'){
    const item=pending.get(message.id);if(!item)return;pending.delete(message.id);
    if(message.success)item.resolve(message.data);else item.reject(new Error(message.error||'操作失败'));
   }else publish(message);
  });
 });
 await waitBounded(ready,10000);
}
app.whenReady().then(async()=>{
 try{
  await startWorker();
  mainWindow=new BrowserWindow({width:1344,height:1000,minWidth:800,minHeight:650,title:'AgentCheck',show:false,backgroundColor:'#ffffff',webPreferences:{preload:path.join(__dirname,'preload.cjs'),contextIsolation:true,nodeIntegration:false,sandbox:true}});
  mainWindow.webContents.setWindowOpenHandler(()=>({action:'deny'}));mainWindow.webContents.on('will-navigate',event=>event.preventDefault());
  mainWindow.on('close',event=>{
   if(quitting)return;event.preventDefault();
   requestQuit();
  });
  await mainWindow.loadFile(path.join(__dirname,'index.html'));await waitBounded(uiReady,10000);mainWindow.show();
  process.stdout.write(JSON.stringify({type:'ready'})+'\n');
  if(process.argv.includes('--agentcheck-handoff')){
   const input=readline.createInterface({input:process.stdin});let prepared=null,started=false;
   input.on('line',async line=>{
    if(started||line.length>1024*1024)return;
    try{
     const value=JSON.parse(line);
     if(value.type==='prepare'&&!prepared){prepared={config:configFromInput(value.config),receipt:value.receipt};process.stdout.write(JSON.stringify({type:'prepared'})+'\n');}
     else if(value.type==='begin'&&prepared){started=true;const ready=prepared;prepared=null;await request('handoff',ready);input.close();}
    }catch(error){publish({type:'error',message:error.message});}
   });
  }
 }catch(error){dialog.showErrorBox('AgentCheck 无法启动',error.message);await killWorkerTree();quitting=true;app.quit();}
});
app.on('before-quit',event=>{if(!quitting){event.preventDefault();requestQuit();}});
app.on('window-all-closed',()=>{if(!quitting)stopAndQuit();});
ipcMain.handle('agentcheck:ready',event=>{if(event.sender===mainWindow?.webContents)uiReadyResolve();return true;});
ipcMain.handle('agentcheck:state',async()=>({...await request('state'),events:[...eventTape],snapshot_sequence:sequence}));
ipcMain.handle('agentcheck:history',()=>request('history'));
ipcMain.handle('agentcheck:result',(_event,id)=>request('result',{run_id:id}));
ipcMain.handle('agentcheck:submit',(_event,input)=>request('submit',{config:configFromInput(input)}));
ipcMain.handle('agentcheck:retest',()=>request('retest'));
ipcMain.handle('agentcheck:cancel',()=>request('cancel'));
ipcMain.handle('agentcheck:report',(_event,id)=>openReport(id));
ipcMain.handle('agentcheck:download',async(_event,id)=>{
 const source=await recordFile(id,'report.html');
 const {canceled,filePath}=await dialog.showSaveDialog(mainWindow,{title:'保存检测报告',defaultPath:`AgentCheck-${id}.html`,filters:[{name:'HTML 报告',extensions:['html']}]});
 if(!canceled&&filePath)await fs.copyFile(source,filePath);return !canceled;
});
