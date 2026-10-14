'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const source=fs.readFileSync(path.join(__dirname,'../desktop/preload.cjs'),'utf8');
test('connection IPC preserves the original multiline error after removing Electron wrapper',async()=>{
 const original='HTTP 401\n{\n  "error": {"message": "vendor detail [REDACTED]", "code": "AUTH_15"}\n}\n';
 let api;
 const context=vm.createContext({require:()=>({contextBridge:{exposeInMainWorld:(_name,value)=>api=value},ipcRenderer:{invoke:async channel=>{throw new Error(`Error invoking remote method '${channel}': Error: ${original}`);}}})});
 vm.runInContext(source,context);
 for(const request of [()=>api.submit({}),()=>api.retest()]){
  await assert.rejects(request,error=>error.message===original);
 }
});
test('non-JSON upstream error text is not parsed or translated by the bridge',async()=>{
 const original='HTTP 502\n<html><body>gateway failure & upstream reset</body></html>\n';
 let api;
 vm.runInNewContext(source,{require:()=>({contextBridge:{exposeInMainWorld:(_name,value)=>api=value},ipcRenderer:{invoke:async()=>{throw new Error(original);}}})});
 await assert.rejects(()=>api.submit({}),error=>error.message===original);
});
