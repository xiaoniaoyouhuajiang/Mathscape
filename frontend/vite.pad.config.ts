import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";
export default defineConfig({
  root:fileURLToPath(new URL("./ink-pad",import.meta.url)),base:"/pad/",
  build:{outDir:fileURLToPath(new URL("./dist-inkpad",import.meta.url)),emptyOutDir:true,
    // The setup guide is a second entry served by the short-lived HTTP helper.
    rolldownOptions:{input:{index:fileURLToPath(new URL("./ink-pad/index.html",import.meta.url)),setup:fileURLToPath(new URL("./ink-pad/setup.html",import.meta.url))}}},
  plugins:[{
    name:"inkpad-offline-assets",
    generateBundle(_,bundle) {
      const urls=["/pad/",...Object.keys(bundle).filter(name=>/\.(?:js|css|woff2?|ttf)$/.test(name)).map(name=>"/pad/"+name)];
      const version="mathscape-inkpad-static-"+Object.keys(bundle).sort().join("-");
      this.emitFile({type:"asset",fileName:"sw.js",source:`const CACHE=${JSON.stringify(version)};const FILES=${JSON.stringify(urls)};self.addEventListener('install',e=>e.waitUntil(caches.open(CACHE).then(c=>c.addAll(FILES)).then(()=>self.skipWaiting())));self.addEventListener('activate',e=>e.waitUntil(caches.keys().then(keys=>Promise.all(keys.filter(k=>k.startsWith('mathscape-inkpad-static-')&&k!==CACHE).map(k=>caches.delete(k)))).then(()=>self.clients.claim())));self.addEventListener('fetch',e=>{const u=new URL(e.request.url);if(e.request.method!=='GET'||u.origin!==self.location.origin||!u.pathname.startsWith('/pad/'))return;e.respondWith(fetch(e.request).catch(()=>caches.match(e.request).then(r=>r||(e.request.mode==='navigate'?caches.match('/pad/'):Response.error()))));});`});
    },
  }],
});
