'use strict';
module.exports=function createReports({BrowserWindow,resolveFile,canAutoOpen,isQuitting,onAutoError}){
 const windows=new Map(),opening=new Map(),attempted=new Set();
 function focus(window){if(window.isMinimized())window.restore();window.show();window.focus();}
 async function create(id){
  const file=await resolveFile(id);
  if(isQuitting())return false;
  const window=new BrowserWindow({width:1280,height:1040,title:'AgentCheck · 检测报告',show:false,webPreferences:{contextIsolation:true,nodeIntegration:false,sandbox:true}});
  windows.set(id,window);
  window.on('closed',()=>{if(windows.get(id)===window)windows.delete(id);});
  window.webContents.setWindowOpenHandler(()=>({action:'deny'}));
  window.webContents.on('will-navigate',event=>event.preventDefault());
  try{
   await window.loadFile(file);
   if(window.isDestroyed())return false;
   if(isQuitting()){window.destroy();return false;}
   focus(window);return true;
  }catch(error){if(!window.isDestroyed())window.destroy();throw error;}
 }
 async function open(id){
  if(isQuitting())return false;
  if(opening.has(id))return opening.get(id);
  const existing=windows.get(id);
  if(existing&&!existing.isDestroyed()){focus(existing);return true;}
  const promise=create(id);opening.set(id,promise);
  try{return await promise;}finally{opening.delete(id);}
 }
 async function finished(run){
  if(!canAutoOpen()||isQuitting()||!run?.id||!run.finished_at||!run.report_path||run.cancelled||attempted.has(run.id))return false;
  attempted.add(run.id);
  try{return await open(run.id);}catch{if(!isQuitting())onAutoError();return false;}
 }
 return {open,finished};
};
