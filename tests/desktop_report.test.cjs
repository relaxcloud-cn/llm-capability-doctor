'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const {createRequire}=require('node:module');
const {EventEmitter}=require('node:events');
const main=path.resolve(__dirname,'../desktop/main.cjs');
const source=fs.readFileSync(main,'utf8');
const moduleRequire=createRequire(main);
const flush=()=>new Promise(resolve=>setImmediate(resolve));
function fixture({env={},args=[],loadError=false}={}){
 const windows=[],messages=[],ipc=new Map();let failLoad=loadError,resolveFile;
 const files={async realpath(file){if(resolveFile)await resolveFile;return file;}};
 class Window extends EventEmitter{
  constructor(options){super();this.options=options;this.shown=0;this.focused=0;this.dead=false;this.webContents={setWindowOpenHandler(){},on(){}};windows.push(this);}
  async loadFile(file){this.file=file;if(failLoad){failLoad=false;throw new Error('private-error-do-not-log');}}
  isDestroyed(){return this.dead;}
  isMinimized(){return false;}
  show(){this.shown++;}
  focus(){this.focused++;}
  destroy(){this.dead=true;this.emit('closed');}
 }
 const app={whenReady:()=>new Promise(()=>{}),on(){},disableHardwareAcceleration(){}};
 const context=vm.createContext({require:name=>name==='electron'?{app,BrowserWindow:Window,ipcMain:{handle:(name,fn)=>ipc.set(name,fn)},dialog:{}}:name==='node:fs/promises'?files:moduleRequire(name),process:{argv:['electron','main.cjs',...args],env,platform:'darwin'},setTimeout,clearTimeout,Buffer,console,uiWindow:{isDestroyed:()=>false,webContents:{send:(channel,message)=>messages.push(message)}}});
 vm.runInContext(source,context,{filename:main});
 vm.runInContext('root="/reports"; mainWindow=uiWindow;',context);
 const finish=async(run={})=>{context.publish({type:'event',data:{event:{type:'finished',run:{id:'AC-test',finished_at:'2026-10-13T12:00:00Z',report_path:'/reports/AC-test/report.html',cancelled:false,verdict:'passed',...run}}}});await flush();};
 return {windows,messages,finish,manual:id=>ipc.get('agentcheck:report')({},id),quitting:()=>vm.runInContext('quitting=true;',context),holdFiles:()=>{let release;resolveFile=new Promise(resolve=>release=resolve);return release;}};
}
test('completion automatically opens the saved HTML for each verdict',async()=>{
 for(const verdict of ['passed','failed','incomplete']){
  const f=fixture();await f.finish({verdict});
  assert.equal(f.windows.length,1);assert.equal(f.windows[0].file,'/reports/AC-test/report.html');
  assert.equal(f.windows[0].options.webPreferences.sandbox,true);
  assert.equal(f.windows[0].options.webPreferences.nodeIntegration,false);
 }
});
test('cancelled, unfinished and missing reports do not auto-open',async()=>{
 for(const run of [{cancelled:true},{finished_at:null},{report_path:null}]){
  const f=fixture();await f.finish(run);assert.equal(f.windows.length,0);
 }
});
test('SSH, CI and explicit opt-out do not auto-open',async()=>{
 for(const name of ['SSH_CONNECTION','SSH_TTY','SSH_CLIENT','CI']){
  const f=fixture({env:{[name]:'1'}});await f.finish();assert.equal(f.windows.length,0);
 }
 const f=fixture({args:['--agentcheck-no-open-report']});await f.finish();assert.equal(f.windows.length,0);
 await f.manual('AC-test');assert.equal(f.windows.length,1);
});
test('duplicate completion and manual clicks share one window, closing allows manual reopening',async()=>{
 const f=fixture();await f.finish();await f.finish();await f.manual('AC-test');assert.equal(f.windows.length,1);
 f.windows[0].destroy();await f.finish();assert.equal(f.windows.length,1);
 await f.manual('AC-test');assert.equal(f.windows.length,2);
 await f.finish({id:'AC-next',report_path:'/reports/AC-next/report.html'});assert.equal(f.windows.length,3);
});
test('simultaneous manual and automatic opening is deduplicated',async()=>{
 const f=fixture();const release=f.holdFiles();await f.finish();const manual=f.manual('AC-test');release();await manual;await flush();assert.equal(f.windows.length,1);
});
test('a failed automatic open preserves results and allows manual retry',async()=>{
 const f=fixture({loadError:true});await f.finish();
 assert.equal(f.windows.length,1);assert(f.windows[0].isDestroyed());
 assert(f.messages.some(message=>message.type==='error'&&message.message.includes('报告已保存')));
 assert(!JSON.stringify(f.messages).includes('private-error-do-not-log'));
 await f.finish();assert.equal(f.windows.length,1);
 await f.manual('AC-test');assert.equal(f.windows.length,2);
});
test('quitting does not create late report windows',async()=>{
 const f=fixture();f.quitting();await f.finish();assert.equal(f.windows.length,0);
 const pending=fixture();const release=pending.holdFiles();await pending.finish();pending.quitting();release();await flush();assert.equal(pending.windows.length,0);
});
