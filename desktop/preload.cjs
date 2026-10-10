'use strict';
const {contextBridge,ipcRenderer}=require('electron');
async function invoke(channel,...args){
 try{return await ipcRenderer.invoke(channel,...args);}
 catch(error){throw new Error(String(error.message||error).replace(/^Error invoking remote method '[^']+': (?:Error: )?/,''));}
}
contextBridge.exposeInMainWorld('agentcheck',{
 ready:()=>invoke('agentcheck:ready'),
 state:()=>invoke('agentcheck:state'),history:()=>invoke('agentcheck:history'),result:id=>invoke('agentcheck:result',id),
 submit:config=>invoke('agentcheck:submit',config),retest:()=>invoke('agentcheck:retest'),cancel:()=>invoke('agentcheck:cancel'),
 report:id=>invoke('agentcheck:report',id),download:id=>invoke('agentcheck:download',id),
 subscribe:listener=>{const receive=(_event,value)=>listener(value);ipcRenderer.on('agentcheck:event',receive);return()=>ipcRenderer.removeListener('agentcheck:event',receive);},
});
