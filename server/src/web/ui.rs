//! Embedded web UI — a single page, vanilla JS against the REST API.

use axum::response::Html;

pub async fn index() -> Html<&'static str> {
    Html(INDEX)
}

const INDEX: &str = r####"<!doctype html>
<html lang="cs">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Evergreen</title>
<style>
  :root { --bg:#0f1115; --card:#181b22; --fg:#e6e8ec; --muted:#9aa4b2; --acc:#4f8cff; --err:#ff5c5c; --ok:#3ecf8e; --line:#262b35; }
  @media (prefers-color-scheme: light) { :root { --bg:#f5f6f8; --card:#fff; --fg:#1a1d23; --muted:#667085; --line:#e4e7ec; } }
  * { box-sizing:border-box; }
  body { margin:0; font:15px/1.5 -apple-system,system-ui,Segoe UI,Roboto,sans-serif; background:var(--bg); color:var(--fg); }
  header { padding:16px 22px; border-bottom:1px solid var(--line); display:flex; align-items:center; gap:12px; }
  header h1 { font-size:18px; margin:0; font-weight:650; }
  header .pill { font-size:12px; color:var(--muted); }
  main { max-width:940px; margin:0 auto; padding:22px; }
  nav { display:flex; gap:6px; margin-bottom:20px; flex-wrap:wrap; }
  nav button { background:transparent; border:1px solid var(--line); color:var(--muted); padding:8px 14px; border-radius:9px; cursor:pointer; font-size:14px; }
  nav button.active { background:var(--acc); color:#fff; border-color:var(--acc); }
  .card { background:var(--card); border:1px solid var(--line); border-radius:12px; padding:18px; margin-bottom:16px; }
  .card h2 { margin:0 0 12px; font-size:15px; }
  .row { display:flex; align-items:center; gap:12px; padding:10px 0; border-bottom:1px solid var(--line); }
  .row:last-child { border-bottom:0; }
  .row .grow { flex:1; min-width:0; }
  .row .name { font-weight:550; }
  .row .sub { font-size:12px; color:var(--muted); word-break:break-all; }
  .icon { width:42px; height:42px; border-radius:9px; background:var(--line); object-fit:cover; flex:none; }
  button.act { background:var(--acc); color:#fff; border:0; padding:7px 12px; border-radius:8px; cursor:pointer; font-size:13px; }
  button.danger { background:transparent; color:var(--err); border:1px solid var(--line); padding:7px 12px; border-radius:8px; cursor:pointer; font-size:13px; }
  input, select { background:var(--bg); color:var(--fg); border:1px solid var(--line); border-radius:8px; padding:9px 11px; font-size:14px; width:100%; }
  label { font-size:13px; color:var(--muted); display:block; margin:10px 0 4px; }
  .muted { color:var(--muted); font-size:13px; }
  .badge { font-size:11px; padding:2px 8px; border-radius:20px; border:1px solid var(--line); }
  .badge.ok { color:var(--ok); border-color:var(--ok); }
  .badge.err { color:var(--err); border-color:var(--err); }
  .drop { border:2px dashed var(--line); border-radius:12px; padding:26px; text-align:center; color:var(--muted); cursor:pointer; }
  .bar { height:6px; background:var(--line); border-radius:4px; overflow:hidden; margin-top:6px; }
  .bar > i { display:block; height:100%; background:var(--acc); width:0; transition:width .3s; }
  .empty { color:var(--muted); text-align:center; padding:22px; }
</style>
</head>
<body>
<header>
  <h1>🌲 Evergreen</h1>
  <span class="pill" id="ver"></span>
  <span class="grow" style="flex:1"></span>
  <span class="pill" id="acct"></span>
</header>
<main>
  <nav>
    <button data-tab="apps" class="active">Aplikace</button>
    <button data-tab="devices">Zařízení</button>
    <button data-tab="jobs">Úlohy</button>
    <button data-tab="account">Účet</button>
  </nav>

  <section id="tab-apps">
    <div class="card">
      <h2>Nahrát IPA</h2>
      <div class="drop" id="drop">Přetáhni sem .ipa nebo klikni pro výběr</div>
      <input type="file" id="file" accept=".ipa" hidden>
      <div class="bar" id="upbar" style="display:none"><i></i></div>
    </div>
    <div class="card">
      <h2>Katalog</h2>
      <div id="apps"></div>
    </div>
  </section>

  <section id="tab-devices" hidden>
    <div class="card">
      <h2>Spárovaná zařízení</h2>
      <p class="muted">Zařízení spáruješ v appce Evergreen (Zařízení → Spárovat iPad) s iPadem připojeným přes USB.</p>
      <div id="devices"></div>
    </div>
  </section>

  <section id="tab-jobs" hidden>
    <div class="card">
      <h2>Úlohy</h2>
      <div id="jobs"></div>
    </div>
  </section>

  <section id="tab-account" hidden>
    <div class="card" id="acctcard">
      <h2>Apple ID</h2>
      <div id="acctbody"></div>
    </div>
  </section>
</main>
<script>
const $ = s => document.querySelector(s);
const api = async (path, opts={}) => {
  const r = await fetch(path, opts);
  if (!r.ok) throw new Error((await r.json().catch(()=>({error:r.statusText}))).error);
  return r.status===204 ? null : r.json();
};
const esc = s => (s||'').replace(/[&<>"]/g, c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));

// tabs
document.querySelectorAll('nav button').forEach(b=>b.onclick=()=>{
  document.querySelectorAll('nav button').forEach(x=>x.classList.toggle('active', x===b));
  ['apps','devices','jobs','account'].forEach(t=>$('#tab-'+t).hidden = t!==b.dataset.tab);
  refresh();
});

async function loadStatus(){
  try { const s=await api('/api/status'); $('#ver').textContent='v'+s.version; } catch{}
  try {
    const a=await api('/api/account');
    $('#acct').textContent = a.linked ? (a.apple_id+' · '+a.auth_state) : 'nepřihlášeno';
  } catch{}
}

async function loadApps(){
  const [apps, devices] = await Promise.all([api('/api/ipa'), api('/api/devices')]);
  const el=$('#apps');
  if(!apps.length){ el.innerHTML='<div class="empty">Zatím žádné IPA</div>'; return; }
  el.innerHTML = apps.map(a=>`
    <div class="row">
      <img class="icon" src="${a.icon_path?('/icon/'+a.id):'data:,'}" onerror="this.style.visibility='hidden'">
      <div class="grow">
        <div class="name">${esc(a.name)} ${a.version?('<span class=muted>'+esc(a.version)+'</span>'):''}</div>
        <div class="sub">${esc(a.bundle_id)}</div>
      </div>
      <select id="dev-${a.id}">${devices.map(d=>`<option value="${d.udid}">${esc(d.name)}</option>`).join('')}</select>
      <button class="act" onclick="install('${a.id}')">Instalovat</button>
      <button class="danger" onclick="delIpa('${a.id}')">×</button>
    </div>`).join('');
}
window.install = async (id)=>{
  const udid = $('#dev-'+id)?.value;
  if(!udid){ alert('Nejdřív spáruj zařízení'); return; }
  try { await api('/api/install',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({device_udid:udid,ipa_id:id})});
    document.querySelector('nav button[data-tab=jobs]').click();
  } catch(e){ alert('Chyba: '+e.message); }
};
window.delIpa = async (id)=>{ if(confirm('Smazat IPA?')){ await api('/api/ipa/'+id,{method:'DELETE'}); loadApps(); } };

async function loadDevices(){
  const d=await api('/api/devices'); const el=$('#devices');
  if(!d.length){ el.innerHTML='<div class="empty">Žádná zařízení</div>'; return; }
  el.innerHTML=d.map(x=>`<div class="row"><div class="grow"><div class="name">${esc(x.name)}</div>
    <div class="sub">${esc(x.udid)} · ${esc(x.address||'IP neznámá')} · iOS ${esc(x.ios_version||'?')}</div></div>
    <button class="danger" onclick="delDev('${x.udid}')">Odebrat</button></div>`).join('');
}
window.delDev = async (u)=>{ if(confirm('Odebrat zařízení?')){ await api('/api/devices/'+u,{method:'DELETE'}); loadDevices(); } };

async function loadJobs(){
  const j=await api('/api/jobs'); const el=$('#jobs');
  if(!j.length){ el.innerHTML='<div class="empty">Žádné úlohy</div>'; return; }
  el.innerHTML=j.map(x=>{
    const b = x.status==='done'?'<span class="badge ok">done</span>':x.status==='error'?'<span class="badge err">error</span>':'<span class="badge">'+x.status+'</span>';
    return `<div class="row"><div class="grow"><div class="name">${esc(x.kind)} #${x.id} ${b}</div>
      <div class="sub">${esc(x.message||'')}</div>
      <div class="bar"><i style="width:${x.progress}%"></i></div></div></div>`;
  }).join('');
}

async function loadAccount(){
  const a=await api('/api/account'); const el=$('#acctbody');
  if(a.linked && a.auth_state==='logged_in'){
    el.innerHTML=`<p>Přihlášeno jako <b>${esc(a.apple_id)}</b>${a.team_id?(' · tým '+esc(a.team_id)):''}</p>
      <button class="danger" onclick="logout()">Odhlásit</button>`;
  } else if(a.auth_state==='needs_2fa'){
    el.innerHTML=`<p class="muted">Zadej 2FA kód z důvěryhodného zařízení:</p>
      <input id="code" placeholder="123456"><button class="act" style="margin-top:10px" onclick="verify()">Ověřit</button>`;
  } else {
    el.innerHTML=`<label>Apple ID</label><input id="aid" placeholder="mail@icloud.com">
      <label>Heslo</label><input id="pw" type="password">
      <button class="act" style="margin-top:12px" onclick="login()">Přihlásit</button>
      <p class="muted" style="margin-top:10px">Heslo se ukládá šifrovaně (AES-256-GCM) a používá se jen k podpisu.</p>`;
  }
}
window.login=async()=>{ try{ await api('/api/account/login',{method:'POST',headers:{'content-type':'application/json'},
  body:JSON.stringify({apple_id:$('#aid').value,password:$('#pw').value})}); loadAccount(); loadStatus(); }catch(e){alert(e.message);} };
window.verify=async()=>{ try{ await api('/api/account/2fa',{method:'POST',headers:{'content-type':'application/json'},
  body:JSON.stringify({code:$('#code').value})}); loadAccount(); loadStatus(); }catch(e){alert(e.message);} };
window.logout=async()=>{ await api('/api/account/logout',{method:'POST'}); loadAccount(); loadStatus(); };

// upload
$('#drop').onclick=()=>$('#file').click();
$('#file').onchange=e=>upload(e.target.files[0]);
$('#drop').ondragover=e=>{e.preventDefault();};
$('#drop').ondrop=e=>{e.preventDefault(); if(e.dataTransfer.files[0]) upload(e.dataTransfer.files[0]);};
async function upload(file){
  if(!file) return;
  const fd=new FormData(); fd.append('file',file);
  $('#upbar').style.display='block'; const bar=$('#upbar>i');
  const xhr=new XMLHttpRequest(); xhr.open('POST','/api/ipa');
  xhr.upload.onprogress=e=>{ bar.style.width=(e.loaded/e.total*100)+'%'; };
  xhr.onload=()=>{ $('#upbar').style.display='none'; bar.style.width='0';
    if(xhr.status<300) loadApps(); else alert('Upload selhal: '+xhr.responseText); };
  xhr.send(fd);
}

function refresh(){
  const tab=document.querySelector('nav button.active').dataset.tab;
  if(tab==='apps') loadApps();
  if(tab==='devices') loadDevices();
  if(tab==='jobs') loadJobs();
  if(tab==='account') loadAccount();
}
loadStatus(); loadApps();
setInterval(()=>{ if(document.querySelector('nav button.active').dataset.tab==='jobs') loadJobs(); }, 2000);
</script>
</body>
</html>"####;
