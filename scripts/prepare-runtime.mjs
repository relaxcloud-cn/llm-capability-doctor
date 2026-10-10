import {createHash} from 'node:crypto';
import {createReadStream,createWriteStream} from 'node:fs';
import {readFile,mkdir,chmod,rename,rm} from 'node:fs/promises';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {Readable} from 'node:stream';
import {pipeline} from 'node:stream/promises';
const root=fileURLToPath(new URL('../',import.meta.url));
const release=JSON.parse(await readFile(resolve(root,'runtime/pi-release.json'),'utf8'));
const targets={'darwin-arm64':'aarch64-apple-darwin','darwin-x64':'x86_64-apple-darwin','linux-arm64':'aarch64-unknown-linux-gnu','linux-x64':'x86_64-unknown-linux-gnu','win32-x64':'x86_64-pc-windows-msvc','win32-arm64':'aarch64-pc-windows-msvc'};
const target=process.argv[2]??targets[`${process.platform}-${process.arch}`];
const asset=release.assets[target];if(!asset)throw new Error('Unsupported target');
const dir=resolve(root,'.cache/omp',release.version,target);await mkdir(dir,{recursive:true});
async function digest(path){const hash=createHash('sha256');for await(const chunk of createReadStream(path))hash.update(chunk);return hash.digest('hex');}
for(const [name,expected,local] of [[asset.name,asset.sha256,target.includes('windows')?'omp.exe':'omp'],['LICENSE',release.licenseSha256,'LICENSE'],['THIRD-PARTY-NOTICES.txt',release.noticesSha256,'THIRD-PARTY-NOTICES.txt']]){
 const path=resolve(dir,local);
 if(await digest(path).catch(()=>null)===expected)continue;
 const temp=path+'.part';
 try{
  const response=await fetch(`https://github.com/${release.repository}/releases/download/v${release.version}/${name}`,{signal:AbortSignal.timeout(600000)});
  if(!response.ok)throw new Error(`Download failed: ${response.status}`);
  await pipeline(Readable.fromWeb(response.body),createWriteStream(temp));
  if(await digest(temp)!==expected)throw new Error('Checksum mismatch');
  await rename(temp,path);
 }finally{await rm(temp,{force:true});}
}
if(!target.includes('windows'))await chmod(resolve(dir,'omp'),0o700);
console.log(`OMP ${release.version} verified: ${dir}`);
