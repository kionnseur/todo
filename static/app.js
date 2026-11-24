(()=>{
function captureShareLink(){const match=location.hash.match(/^#partage=([A-Za-z0-9_-]{43})$/);if(!match)return null;sessionStorage.setItem('todoPendingShare',match[1]);history.replaceState(history.state,'',location.pathname+location.search);return match[1]}
captureShareLink();
const root=document.getElementById('tree'); if(!root)return;
const panel=root.closest('.panel'), heading=panel.querySelector('.heading'), modal=document.getElementById('modal'), form=document.getElementById('task-form');
let tasks=[],tasksById=new Map(),childrenByParent=new Map(),searchIndex=[],sort='custom',editingOrder=false,selected=null,menuId=null,carouselPath=[],carouselLevel=0,timer,refreshSeq=0,mutationCount=0,interactionVersion=0,scrollHistoryTimer,restoringHistory=false;
const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const api=async(u,o={})=>{const mutation=o.method&&o.method!=='GET';if(mutation){mutationCount++;refreshSeq++}try{const r=await fetch(u,{credentials:'same-origin',headers:{'Content-Type':'application/json','X-Todo-Request':'1'},...o});if(r.status===401){location.href='/auth/login';throw Error('Session expirée')}if(!r.ok){const d=await r.json().catch(()=>({}));throw Error(d.error||'Une erreur est survenue')}return r.status===204?null:r.json()}finally{if(mutation)mutationCount--}};
function notify(message){let t=document.querySelector('.toast');if(!t){t=document.createElement('div');t.className='toast';document.body.append(t)}t.textContent=message;clearTimeout(timer);timer=setTimeout(()=>t.remove(),3500)}
function colorIndex(task){const index=Number(task.category_color);return Number.isInteger(index)&&index>=1&&index<=10?index:0}
function toolbarIcon(name){const paths={search:'<circle cx="10.5" cy="10.5" r="6.5"/><path d="m15.5 15.5 5 5"/>',link:'<path d="m10 13 4-4m-6 2-2 2a4.25 4.25 0 0 0 6 6l2-2m-4-10 2-2a4.25 4.25 0 0 1 6 6l-2 2"/>',activity:'<circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3 2"/>'};return `<svg class="toolbar-icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">${paths[name]}</svg>`}
function normalizeSearch(value){return String(value??'').normalize('NFKD').replace(/\p{M}/gu,'').toLocaleLowerCase('fr-FR').replace(/œ/g,'oe').replace(/æ/g,'ae').replace(/[’‘]/g,"'")}
// Le menu vit hors du conteneur défilant pour ne jamais être rogné.
const taskMenu=document.createElement('div');taskMenu.id='task-actions';taskMenu.className='task-menu hidden';taskMenu.setAttribute('role','menu');taskMenu.innerHTML='<button type="button" role="menuitem" data-act="edit">Modifier</button><button type="button" role="menuitem" data-act="share">Partager</button><button type="button" role="menuitem" data-act="up">↑ Monter</button><button type="button" role="menuitem" data-act="down">↓ Descendre</button><button type="button" role="menuitem" data-act="delete" class="danger">Supprimer</button>';document.body.append(taskMenu);
function menuTrigger(){return menuId?root.querySelector(`[data-id="${CSS.escape(menuId)}"] .menu-toggle`):null}
function closeTaskMenu(restoreFocus=false){const trigger=menuTrigger();trigger?.setAttribute('aria-expanded','false');menuId=null;taskMenu.classList.add('hidden');if(restoreFocus)trigger?.focus({preventScroll:true})}
function positionTaskMenu(){
  const trigger=menuTrigger();if(!trigger)return;
  const anchor=trigger.getBoundingClientRect(),viewport=window.visualViewport,edge=8,left=viewport?.offsetLeft||0,top=viewport?.offsetTop||0,width=viewport?.width||document.documentElement.clientWidth,height=viewport?.height||window.innerHeight;
  taskMenu.style.maxHeight=`${Math.max(0,height-edge*2)}px`;
  if(anchor.bottom<top||anchor.top>top+height||anchor.right<left||anchor.left>left+width){closeTaskMenu();return}
  const menuWidth=taskMenu.offsetWidth,below=top+height-anchor.bottom-edge,above=anchor.top-top-edge,opensUp=below<taskMenu.offsetHeight+6&&above>below;
  taskMenu.style.maxHeight=`${Math.max(0,(opensUp?above:below)-6)}px`;
  const menuHeight=taskMenu.offsetHeight;
  taskMenu.style.left=`${Math.max(left+edge,Math.min(anchor.right-menuWidth,left+width-menuWidth-edge))}px`;
  taskMenu.style.top=`${Math.max(top+edge,Math.min(opensUp?anchor.top-menuHeight-6:anchor.bottom+6,top+height-menuHeight-edge))}px`;
  taskMenu.dataset.side=opensUp?'top':'bottom';
}
function toggleTaskMenu(id){if(menuId===id){closeTaskMenu();return}closeTaskMenu();menuId=id;taskMenu.dataset.taskId=id;taskMenu.setAttribute('aria-label',`Actions de ${tasksById.get(id).title}`);menuTrigger()?.setAttribute('aria-expanded','true');taskMenu.classList.remove('hidden');positionTaskMenu();taskMenu.querySelector('button').focus({preventScroll:true})}
document.addEventListener('pointerdown',e=>{if(menuId&&!taskMenu.contains(e.target)&&!e.target.closest('.menu-toggle'))closeTaskMenu()});
taskMenu.addEventListener('keydown',e=>{const buttons=[...taskMenu.querySelectorAll('button')],index=buttons.indexOf(document.activeElement);if(['ArrowDown','ArrowUp','Home','End'].includes(e.key)){e.preventDefault();buttons[e.key==='Home'?0:e.key==='End'?buttons.length-1:(index+(e.key==='ArrowUp'?-1:1)+buttons.length)%buttons.length].focus()}else if(e.key==='Tab')closeTaskMenu(true)});
window.addEventListener('scroll',()=>{if(menuId)positionTaskMenu()},{passive:true});
window.visualViewport?.addEventListener('resize',()=>{if(menuId)positionTaskMenu()});
window.visualViewport?.addEventListener('scroll',()=>{if(menuId)positionTaskMenu()});
const controls=document.createElement('div');controls.className='heading-actions';controls.innerHTML='<button type="button" class="view-action" id="edit-order" aria-label="Modifier l’ordre" aria-pressed="false" title="Afficher les numéros de position">✎</button><button type="button" class="view-action" id="group-category" aria-label="Regrouper par catégorie" aria-pressed="false" title="Regrouper par catégorie">☷</button>';heading.insertBefore(controls,document.getElementById('new-task'));controls.append(document.getElementById('new-task'));
const activityButton=document.createElement('button');activityButton.type='button';activityButton.className='view-action';activityButton.title='Activité automatique';activityButton.setAttribute('aria-label','Activité automatique');activityButton.innerHTML=toolbarIcon('activity');controls.prepend(activityButton);
const toTop=document.createElement('button');toTop.type='button';toTop.className='scroll-top';toTop.textContent='↑';toTop.setAttribute('aria-label','Remonter en haut de la page');toTop.hidden=true;document.body.append(toTop);
window.addEventListener('scroll',()=>{toTop.hidden=window.scrollY<300},{passive:true});toTop.onclick=()=>window.scrollTo({top:0,behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth'});
const activityOverlay=document.createElement('div');activityOverlay.className='overlay hidden';activityOverlay.innerHTML='<section class="modal activity-modal" role="dialog" aria-modal="true" aria-labelledby="activity-title"><div class="activity-header"><h2 id="activity-title">Activité automatique</h2><button type="button" class="iconbtn" aria-label="Fermer">×</button></div><div class="activity-tabs" role="tablist" aria-label="Période de l’activité"><button type="button" role="tab" id="activity-upcoming-tab" data-view="upcoming" aria-selected="true" aria-controls="activity-panel">À venir</button><button type="button" role="tab" id="activity-history-tab" data-view="history" aria-selected="false" aria-controls="activity-panel" tabindex="-1">Historique</button></div><div class="activity-list" id="activity-panel" role="tabpanel" aria-labelledby="activity-upcoming-tab" aria-live="polite" tabindex="0"></div></section>';document.body.append(activityOverlay);
let activityView='upcoming',activityRequest=0;
function closeActivity(){activityRequest++;activityOverlay.classList.add('hidden')}
activityOverlay.querySelector('.activity-header button').onclick=closeActivity;activityOverlay.onclick=e=>{if(e.target===activityOverlay)closeActivity()};
function activityDate(timestamp,timezone){return new Date(timestamp*1000).toLocaleString('fr-FR',{dateStyle:'medium',timeStyle:'short',...(timezone?{timeZone:timezone}:{})})}
async function loadActivity(view){
  activityView=view;const request=++activityRequest,list=activityOverlay.querySelector('.activity-list');
  activityOverlay.querySelectorAll('[data-view]').forEach(button=>{const active=button.dataset.view===view;button.setAttribute('aria-selected',String(active));button.tabIndex=active?0:-1});
  list.setAttribute('aria-labelledby',`activity-${view}-tab`);list.setAttribute('aria-busy','true');list.textContent='Chargement…';
  try{
    const result=await api(view==='upcoming'?'/api/activity/upcoming':'/api/activity');if(request!==activityRequest)return;
    const upcoming=view==='upcoming',events=upcoming?result.events:result;
    list.innerHTML=upcoming?`<p class="activity-note">Prévisions selon l’état actuel des tâches. Les dates évoluent si tu les modifies.<br>Fuseau : ${esc(result.timezone)}.</p>`:'';
    if(!events.length){list.insertAdjacentHTML('beforeend',`<p class="modal-empty">${upcoming?'Aucune action automatique prévue. Les tâches récurrentes et les tâches cochées à supprimer apparaîtront ici.':'Aucune activité automatique pour le moment.'}</p>`);return}
    list.insertAdjacentHTML('beforeend',events.map(event=>{const timestamp=upcoming?event.scheduled_at:event.occurred_at,reset=event.kind==='reset',count=event.affected_count||0;const detail=upcoming?(reset?(count?`${count} tâche${count>1?'s cochées seront décochées':' cochée sera décochée'}.`:'Les tâches cochées seront décochées à cette date.'):`${count} tâche${count>1?'s seront supprimées, sous-tâches comprises':' sera supprimée'}. Décoche la tâche pour annuler sa suppression.`):'';return `<article class="activity-event" data-kind="${reset?'reset':'deleted'}"><time datetime="${new Date(timestamp*1000).toISOString()}">${esc(activityDate(timestamp,upcoming?result.timezone:null))}</time>${upcoming&&timestamp*1000<=Date.now()?'<span class="activity-soon">Imminent</span>':''}<p>${reset?'Réinitialisation de période':'Suppression automatique'} : <strong>${esc(event.title)}</strong></p>${detail?`<small>${esc(detail)}</small>`:''}</article>`}).join(''));
  }catch(error){if(request===activityRequest)list.innerHTML=`<p>${esc(error.message)}</p><button type="button" class="muted" data-retry-activity>Réessayer</button>`}
  finally{if(request===activityRequest)list.setAttribute('aria-busy','false')}
}
activityOverlay.querySelector('.activity-tabs').onclick=e=>{const button=e.target.closest('[data-view]');if(button)loadActivity(button.dataset.view)};
activityOverlay.querySelector('.activity-tabs').onkeydown=e=>{const tabs=[...activityOverlay.querySelectorAll('[data-view]')],index=tabs.indexOf(document.activeElement);if(!['ArrowLeft','ArrowRight','Home','End'].includes(e.key))return;e.preventDefault();const next=tabs[e.key==='Home'?0:e.key==='End'?tabs.length-1:(index+(e.key==='ArrowLeft'?-1:1)+tabs.length)%tabs.length];next.focus();loadActivity(next.dataset.view)};
activityOverlay.querySelector('.activity-list').onclick=e=>{if(e.target.closest('[data-retry-activity]'))loadActivity(activityView)};
activityButton.onclick=()=>{closeTaskMenu();activityOverlay.classList.remove('hidden');activityOverlay.querySelector('.activity-header button').focus();loadActivity('upcoming')};
const categoryOverlay=document.createElement('div');categoryOverlay.className='overlay hidden';categoryOverlay.innerHTML='<form class="modal category-modal" role="dialog" aria-modal="true" aria-labelledby="category-title"><h2 id="category-title">Catégorie</h2><label class="field">Nom de la catégorie<input class="fieldinput" name="category" list="quick-category-suggestions" maxlength="40" autocomplete="off"></label><datalist id="quick-category-suggestions"></datalist><div class="modal-actions"><button type="button" class="muted">Annuler</button><button class="primary" type="submit">Enregistrer</button></div></form>';document.body.append(categoryOverlay);
let categoryTaskId=null;const categoryForm=categoryOverlay.querySelector('form');
function closeCategory(){categoryOverlay.classList.add('hidden')}
categoryOverlay.querySelector('.muted').onclick=closeCategory;categoryOverlay.onclick=e=>{if(e.target===categoryOverlay)closeCategory()};
function showCategory(task){categoryTaskId=task.id;categoryForm.elements.category.value=task.category;categoryForm.querySelector('datalist').innerHTML=[...new Set(tasks.map(t=>t.category).filter(Boolean))].sort((a,b)=>a.localeCompare(b)).map(c=>`<option value="${esc(c)}"></option>`).join('');categoryOverlay.classList.remove('hidden');categoryForm.elements.category.focus()}
categoryForm.onsubmit=async e=>{e.preventDefault();const button=categoryForm.querySelector('[type=submit]');button.disabled=true;try{await api(`/api/tasks/${encodeURIComponent(categoryTaskId)}/category`,{method:'PATCH',body:JSON.stringify({category:categoryForm.elements.category.value.trim().toLowerCase()})});closeCategory();await refresh()}catch(error){notify(error.message)}finally{button.disabled=false}};
const searchButton=document.createElement('button');searchButton.type='button';searchButton.className='view-action';searchButton.setAttribute('aria-label','Rechercher une tâche');searchButton.title='Rechercher une tâche';searchButton.innerHTML=toolbarIcon('search');controls.prepend(searchButton);
const searchOverlay=document.createElement('div');searchOverlay.className='overlay hidden';searchOverlay.innerHTML='<section class="modal search-modal" role="dialog" aria-modal="true" aria-labelledby="search-title"><div class="activity-header"><h2 id="search-title">Rechercher une tâche</h2><button type="button" class="iconbtn" aria-label="Fermer">×</button></div><input class="fieldinput" type="search" placeholder="Titre, description ou catégorie" aria-label="Rechercher"><div class="search-results" aria-live="polite"></div></section>';document.body.append(searchOverlay);
const searchInput=searchOverlay.querySelector('input'),searchResults=searchOverlay.querySelector('.search-results');
const shareOverlay=document.createElement('div');shareOverlay.className='overlay hidden';shareOverlay.innerHTML='<section class="modal share-modal" role="dialog" aria-modal="true" aria-labelledby="share-title"><div class="activity-header"><h2 id="share-title">Partager</h2><button type="button" class="iconbtn" aria-label="Fermer">×</button></div><div class="share-content"></div><div class="modal-actions"></div></section>';document.body.append(shareOverlay);
const shareTitle=shareOverlay.querySelector('h2'),shareContent=shareOverlay.querySelector('.share-content'),shareActions=shareOverlay.querySelector('.modal-actions');
let shareRequest=0;
function beginShare(title){const request=++shareRequest;closeTaskMenu();shareTitle.textContent=title;shareContent.innerHTML='<p>Chargement…</p>';shareContent.setAttribute('aria-busy','true');shareActions.innerHTML='';shareOverlay.classList.remove('hidden');shareOverlay.querySelector('.activity-header button').focus();return request}
function closeShare(){shareRequest++;shareOverlay.classList.add('hidden');sessionStorage.removeItem('todoPendingShare')}
shareOverlay.querySelector('.iconbtn').onclick=closeShare;shareOverlay.onclick=e=>{if(e.target===shareOverlay)closeShare()};
function showShareLink(task){
  const request=beginShare('Partager la tâche');shareContent.setAttribute('aria-busy','false');
  shareContent.innerHTML=`<p>Partager une copie figée de <strong>${esc(task.title)}</strong> et de ses sous-tâches. Les modifications ultérieures ne seront pas synchronisées.</p><label class="field">Durée de validité<select class="fieldinput share-duration"><option value="86400">1 jour</option><option value="604800">1 semaine</option><option value="">Sans expiration</option></select></label>`;
  shareActions.innerHTML='<button type="button" class="muted">Annuler</button><button type="button" class="primary">Créer le lien</button>';shareActions.querySelector('.muted').onclick=closeShare;
  shareActions.querySelector('.primary').onclick=async e=>{
    const button=e.currentTarget,duration=shareContent.querySelector('.share-duration').value;button.disabled=true;
    try{
      const result=await api(`/api/tasks/${encodeURIComponent(task.id)}/share`,{method:'POST',body:JSON.stringify({duration_seconds:duration?Number(duration):null})});if(request!==shareRequest)return;
      const url=new URL('/',location.origin);url.hash=`partage=${result.token}`;
      shareContent.innerHTML=`<p>Le lien est prêt. Toute personne connectée qui le reçoit pourra importer cette copie.</p><input class="fieldinput share-link" readonly aria-label="Lien de partage"><p class="share-expiry">${result.expires_at?`Expire le ${esc(activityDate(result.expires_at))}.`:'Sans expiration automatique.'}</p>`;
      const input=shareContent.querySelector('input');input.value=url.href;shareActions.innerHTML='<button type="button" class="primary share-copy">Copier le lien</button>';
      shareActions.querySelector('.share-copy').onclick=async()=>{try{await navigator.clipboard.writeText(input.value);notify('Lien copié')}catch{input.focus();input.select();notify('Sélectionne et copie le lien')}};
      if(typeof navigator.share==='function'){
        const native=document.createElement('button');native.type='button';native.className='muted';native.textContent='Partager';
        native.onclick=async()=>{try{await navigator.share({title:task.title,url:input.value})}catch(error){if(error.name!=='AbortError')notify('Partage indisponible : copie le lien')}};shareActions.append(native);
      }
      shareActions.querySelector('.share-copy').focus();
    }catch(error){if(request===shareRequest){notify(error.message);button.disabled=false}}
  };
}
const manageSharesButton=document.createElement('button');manageSharesButton.type='button';manageSharesButton.className='view-action';manageSharesButton.setAttribute('aria-label','Gérer les liens de partage');manageSharesButton.title='Gérer les liens de partage';manageSharesButton.innerHTML=toolbarIcon('link');controls.prepend(manageSharesButton);
async function showSharedLinks(){
  const request=beginShare('Liens de partage');
  try{
    const links=await api('/api/shares');if(request!==shareRequest)return;
    shareContent.innerHTML='<p>Ces liens donnent accès à une copie figée des tâches. Désactiver un lien empêche de nouveaux imports, sans supprimer les copies déjà importées.</p>';
    shareContent.insertAdjacentHTML('beforeend',links.length?'<div class="share-list managed-shares"></div>':'<p class="modal-empty">Aucun lien de partage. Utilise « Partager » dans le menu d’une tâche pour en créer un.</p>');
    const list=shareContent.querySelector('.share-list');
    for(const link of links){
      const expired=link.expires_at!=null&&link.expires_at*1000<=Date.now(),item=document.createElement('article');item.className='share-item share-managed';item.dataset.shareId=link.id;
      item.innerHTML=`<div class="share-managed-info"><div class="share-managed-heading"><strong>${esc(link.title||'Tâche partagée')}</strong><span class="share-status ${expired?'is-expired':''}">${expired?'Expiré':'Actif'}</span></div>${link.created_at>0?`<small>Créé le ${esc(activityDate(link.created_at))}</small>`:''}<small>${link.expires_at!=null?`${expired?'Expiré':'Expire'} le ${esc(activityDate(link.expires_at))}`:'Sans expiration'}</small>${Number.isInteger(link.task_count)?`<small>${link.task_count} tâche${link.task_count>1?'s':''} dans la copie</small>`:''}</div><button type="button" class="muted">${expired?'Supprimer':'Désactiver'}</button>`;
      item.querySelector('button').onclick=async e=>{
        if(!confirm(expired?'Supprimer ce lien expiré de la liste ?':'Désactiver ce lien de partage ? Les copies déjà importées seront conservées.'))return;
        const button=e.currentTarget;button.disabled=true;
        try{await api(`/api/shares/manage/${encodeURIComponent(link.id)}`,{method:'DELETE'});if(request===shareRequest){await showSharedLinks();notify(expired?'Lien expiré supprimé':'Lien désactivé')}}catch(error){if(request===shareRequest){notify(error.message);button.disabled=false}}
      };
      list.append(item);
    }
  }catch(error){if(request===shareRequest){shareContent.innerHTML=`<p>${esc(error.message)}</p><button type="button" class="muted">Réessayer</button>`;shareContent.querySelector('button').onclick=showSharedLinks}}
  finally{if(request===shareRequest)shareContent.setAttribute('aria-busy','false')}
}
manageSharesButton.onclick=showSharedLinks;
async function showSharePreview(token){
  const request=beginShare('Importer les tâches');
  try{
    const preview=await api(`/api/shares/${encodeURIComponent(token)}`);if(request!==shareRequest)return;const shared=preview.tasks,depths=new Map();
    shareContent.innerHTML='<p>Une copie de ces tâches sera ajoutée à ta liste, sans les cocher.</p><label class="field">Importer sous<select class="fieldinput share-destination"><option value="">À la racine</option><option value="__new__">Créer une nouvelle tâche parente…</option></select></label><label class="field share-new-parent hidden">Nom de la nouvelle tâche parente<input class="fieldinput" maxlength="160"></label><div class="share-list"></div>';
    if(preview.already_imported){const warning=document.createElement('p');warning.className='share-warning';warning.textContent='Tu as déjà importé ce partage. Tu peux l’ajouter à nouveau.';shareContent.prepend(warning)}
    const select=shareContent.querySelector('.share-destination'),newParent=shareContent.querySelector('.share-new-parent'),parentInput=newParent.querySelector('input');
    const choices=tasks.map(task=>{const path=[task.title];let parent=tasksById.get(task.parent_id);while(parent){path.unshift(parent.title);parent=tasksById.get(parent.parent_id)}return {id:task.id,label:path.join(' › ')}}).sort((a,b)=>a.label.localeCompare(b.label));
    for(const choice of choices){const option=document.createElement('option');option.value=choice.id;option.textContent=choice.label;select.append(option)}
    select.onchange=()=>{newParent.classList.toggle('hidden',select.value!=='__new__');parentInput.setCustomValidity('');if(select.value==='__new__')parentInput.focus()};parentInput.oninput=()=>parentInput.setCustomValidity('');
    const list=shareContent.querySelector('.share-list');for(const task of shared){const depth=task.parent_id?(depths.get(task.parent_id)||0)+1:0;depths.set(task.id,depth);const item=document.createElement('div');item.className='share-item';item.style.paddingInlineStart=`${Math.min(depth,8)*18}px`;item.innerHTML=`<strong>${esc(task.title)}</strong>${task.category?`<small>${esc(task.category)}</small>`:''}${task.recurrence&&task.recurrence!=='none'?recurrenceIcon(task.recurrence):''}${task.description?`<p>${esc(task.description)}</p>`:''}`;list.append(item)}
    shareActions.innerHTML='<button type="button" class="muted">Annuler</button><button type="button" class="primary">Ajouter à mes tâches</button>';shareActions.querySelector('.muted').onclick=closeShare;
    shareActions.querySelector('.primary').onclick=async e=>{const button=e.currentTarget,isNew=select.value==='__new__',title=parentInput.value.trim();if(isNew&&!title){parentInput.setCustomValidity('Saisis un titre contenant au moins un caractère.');parentInput.reportValidity();return}const destination=isNew?{new_parent_title:title}:{parent_id:select.value||null};button.disabled=true;try{await api(`/api/shares/${encodeURIComponent(token)}`,{method:'POST',body:JSON.stringify(destination)});if(request===shareRequest)closeShare();await refresh();notify('Tâches ajoutées')}catch(error){if(request===shareRequest){notify(error.message);button.disabled=false}}};
    select.focus();
  }catch(error){if(request===shareRequest)shareContent.textContent=error.message}
  finally{if(request===shareRequest)shareContent.setAttribute('aria-busy','false')}
}
function closeSearch(){searchOverlay.classList.add('hidden')}
searchOverlay.querySelector('button').onclick=closeSearch;searchOverlay.onclick=e=>{if(e.target===searchOverlay)closeSearch()};
function rebuildSearchIndex(){searchIndex=tasks.map(task=>{const path=[];let parent=tasksById.get(task.parent_id);while(parent){path.unshift(parent.title);parent=tasksById.get(parent.parent_id)}return {task,path:path.join(' › '),title:normalizeSearch(task.title),text:normalizeSearch([task.title,task.description,task.category,...path].join(' '))}})}
function updateSearch(){
  const query=normalizeSearch(searchInput.value).trim(),terms=query.split(/\s+/);
  if(!query){searchResults.innerHTML='<p>Saisis un ou plusieurs mots pour rechercher, avec ou sans accents.</p>';return}
  const score=entry=>entry.title===query?0:entry.title.startsWith(query)?1:terms.every(term=>entry.title.includes(term))?2:3;
  const matches=searchIndex.filter(entry=>terms.every(term=>entry.text.includes(term))).sort((a,b)=>score(a)-score(b));
  searchResults.innerHTML=matches.length?`<p class="search-summary">${matches.length} résultat${matches.length>1?'s':''}</p>`+matches.map(({task,path})=>`<button type="button" data-search-id="${esc(task.id)}"><strong>${esc(task.title)}</strong>${path?`<small>${esc(path)}</small>`:''}${task.category?`<small>${esc(task.category)}</small>`:''}</button>`).join(''):'<p>Aucune tâche trouvée.</p>';
}
searchInput.oninput=updateSearch;searchButton.onclick=()=>{closeTaskMenu();searchInput.value='';updateSearch();searchOverlay.classList.remove('hidden');searchInput.focus()};
searchInput.onkeydown=e=>{if(!['ArrowDown','Enter'].includes(e.key)||e.isComposing)return;const first=searchResults.querySelector('[data-search-id]');if(!first)return;e.preventDefault();if(e.key==='Enter')first.click();else first.focus()};
searchResults.onkeydown=e=>{if(!['ArrowDown','ArrowUp','Home','End'].includes(e.key))return;e.preventDefault();const buttons=[...searchResults.querySelectorAll('[data-search-id]')],index=buttons.indexOf(document.activeElement);if(e.key==='ArrowUp'&&index===0){searchInput.focus();return}buttons[e.key==='Home'?0:e.key==='End'?buttons.length-1:(index+(e.key==='ArrowUp'?-1:1)+buttons.length)%buttons.length]?.focus()};
searchResults.onclick=e=>{const id=e.target.closest('[data-search-id]')?.dataset.searchId,task=tasksById.get(id);if(!task)return;const path=[];let parent=tasksById.get(task.parent_id);while(parent){path.unshift(parent.id);parent=tasksById.get(parent.parent_id)}carouselPath=path;selected=id;if(childrenByParent.has(id))carouselPath.push(id);closeTaskMenu();closeSearch();render();requestAnimationFrame(()=>{scrollToPage(path.length,'instant');const target=[...root.querySelectorAll('.task')].find(node=>node.dataset.id===id);target?.scrollIntoView({block:'center',inline:'nearest',behavior:'instant'});target?.querySelector('.task-name')?.focus({preventScroll:true})})};
const fields=document.createElement('div');fields.className='form-grid';fields.innerHTML='<label class="field">Catégorie <input class="fieldinput" name="category" list="category-suggestions" maxlength="40" placeholder="Ex. maison"><datalist id="category-suggestions"></datalist></label><label class="field">Position <input class="fieldinput" name="position" type="number" min="0" placeholder="Auto"></label>';form.querySelector('.modal-actions').before(fields);const recurrence=document.createElement('label');recurrence.className='field';recurrence.innerHTML='Récurrence <select class="fieldinput" name="recurrence"><option value="none">Aucune</option><option value="daily">Chaque jour</option><option value="weekly">Chaque semaine</option><option value="monthly">Chaque mois</option><option value="quarterly">Chaque trimestre</option><option value="yearly">Chaque année</option></select>';form.querySelector('.modal-actions').before(recurrence);
function ordered(list){return [...list].sort((a,b)=>sort==='category'?a.category.localeCompare(b.category)||a.sort_order-b.sort_order:a.sort_order-b.sort_order)}
function grouped(list,renderItem){let previous;return ordered(list).map(t=>{const label=sort==='category'&&t.category!==previous?`<div class="category-heading">${esc(t.category||'Sans catégorie')}</div>`:'';previous=t.category;return label+renderItem(t)}).join('')}
function recurrenceIcon(value){const periods={daily:['J','Chaque jour'],weekly:['S','Chaque semaine'],monthly:['M','Chaque mois'],quarterly:['T','Chaque trimestre'],yearly:['A','Chaque année']},period=periods[value];return period?`<span class="recurrence-icon" role="img" aria-label="${period[1]}" title="${period[1]}"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M19 8a8 8 0 0 0-13-2L4 8m0 0V4m0 4h4M5 16a8 8 0 0 0 13 2l2-2m0 0v4m0-4h-4"/></svg><span>${period[0]}</span></span>`:''}
function taskAgeIcon(t){
  if(t.completed_at||(t.recurrence&&t.recurrence!=='none')||!Number.isFinite(t.created_at))return '';
  const created=new Date(t.created_at*1000),threshold=new Date(created),day=created.getDate();
  // Un mois calendaire : le 31 janvier atteint le seuil le dernier jour de février.
  threshold.setDate(1);threshold.setMonth(threshold.getMonth()+1);threshold.setDate(Math.min(day,new Date(threshold.getFullYear(),threshold.getMonth()+1,0).getDate()));
  if(!Number.isFinite(threshold.getTime())||Date.now()<threshold.getTime())return '';
  const label=`En attente depuis au moins un mois · Créée le ${created.toLocaleDateString('fr-FR',{day:'numeric',month:'long',year:'numeric'})}`;
  return `<span class="task-age" role="img" aria-label="${esc(label)}" title="${esc(label)}"><svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="6.5"/><path d="M10 6v4l2.5 1.5"/></svg></span>`;
}
function updateTaskAges(){for(const node of root.querySelectorAll('.task')){const task=tasksById.get(node.dataset.id);if(!task)continue;const existing=node.querySelector('.task-age'),icon=taskAgeIcon(task);if(!icon)existing?.remove();else if(!existing)node.querySelector('.task-name').insertAdjacentHTML('afterend',icon)}}
function row(t){const has=childrenByParent.has(t.id),expanded=selected===t.id||carouselPath.includes(t.id);return `<div class="task-row" draggable="${sort==='custom'}">${editingOrder?`<div class="position-control"><input class="position-input" data-act="position" type="number" min="0" value="${t.sort_order}" aria-label="Position de ${esc(t.title)}"><div class="position-buttons"><button type="button" data-act="position-minus" aria-label="Diminuer la position de ${esc(t.title)}">▲</button><button type="button" data-act="position-plus" aria-label="Augmenter la position de ${esc(t.title)}">▼</button></div></div>`:''}<input class="check" type="checkbox" data-act="check" aria-label="Terminer ${esc(t.title)}" ${t.completed_at?'checked':''}><button type="button" class="category-button" data-act="category" aria-label="Modifier la cat\u00e9gorie de ${esc(t.title)}">${t.category?`<span class="category-dot category-color-${colorIndex(t)}"></span>`:'<span class="category-dot category-empty"></span>'}</button><button class="task-name" data-act="expand" aria-expanded="${expanded}">${esc(t.title)}${t.category?`<small class="category-label">${esc(t.category)}</small>`:''}</button>${taskAgeIcon(t)}${recurrenceIcon(t.recurrence)}<button class="iconbtn" data-act="add-child" aria-label="Ajouter une sous-tâche">+</button><div class="menu-wrap"><button class="iconbtn menu-toggle" data-act="menu" aria-label="Actions" aria-haspopup="menu" aria-controls="task-actions" aria-expanded="${menuId===t.id}">⋯</button></div>${has?`<button class="iconbtn expand-toggle ${expanded?'points-right':''}" data-act="expand" aria-label="Afficher les détails" aria-expanded="${expanded}"><span class="twisty">▲</span></button>`:'<span class="arrow-spacer" aria-hidden="true"></span>'}</div>`}
function desc(t){return `<p class="desc ${t?.description?'':'none'}">${esc(t?.description||'Aucune description')}</p>`}
function dots(count,active){return `<nav class="page-dots" aria-label="Navigation des pages">${Array.from({length:count},(_,i)=>`<button type="button" data-page-dot="${i}" aria-label="Page ${i+1}" ${i===active?'aria-current="page"':''}></button>`).join('')}</nav>`}
const navigationId=Date.now().toString(36)+Math.random().toString(36).slice(2);
let historyLevel=0;
history.replaceState({...history.state,todoNavigation:navigationId,todoLevel:0,todoPrevious:null},'');
function alignCarouselPages(){
  const track=root.querySelector('.levels-track');if(!track)return;
  [...track.children].forEach((page,index)=>{const previous=track.children[index-1],source=previous?.querySelector(`[data-id="${CSS.escape(carouselPath[index-1]||'')}"]`),offset=source?previous.offsetTop+source.offsetTop:0;page.style.marginTop=`${offset}px`});
}
function updateTrackHeight(){
  const track=root.querySelector('.levels-track');if(!track)return;
  const bounds=track.getBoundingClientRect();let height=0;
  for(const page of track.children){const rect=page.getBoundingClientRect();if(rect.right>bounds.left+1&&rect.left<bounds.right-1)height=Math.max(height,page.offsetTop+page.offsetHeight)}
  if(height)track.style.height=`${height}px`;
}
const pageResize=new ResizeObserver(()=>{alignCarouselPages();updateTrackHeight();if(menuId)positionTaskMenu()});
function observePages(){pageResize.disconnect();root.querySelectorAll('.level-page').forEach(page=>pageResize.observe(page))}
let taskSwipe=null,carouselTarget=null,carouselSettleTimer;
// Verrouiller l'axe dès le début du geste : défilement vertical natif, swipe horizontal volontaire.
root.addEventListener('touchstart',e=>{
  if(e.touches.length!==1)return;
  const track=e.target.closest('.levels-track');if(!track)return;
  clearTimeout(carouselSettleTimer);carouselTarget=null;track.classList.add('is-swiping');track.scrollTo({left:track.scrollLeft,behavior:'instant'});
  const touch=e.touches[0];taskSwipe={track,id:touch.identifier,x:touch.clientX,y:touch.clientY,left:track.scrollLeft,level:carouselLevel,started:performance.now(),axis:null,dx:0};
},{passive:true});
root.addEventListener('touchmove',e=>{
  if(!taskSwipe)return;
  if(e.touches.length!==1){finishTaskSwipe(true);return}
  const touch=[...e.touches].find(touch=>touch.identifier===taskSwipe.id);if(!touch)return;
  const dx=touch.clientX-taskSwipe.x,dy=touch.clientY-taskSwipe.y;
  if(!taskSwipe.axis&&Math.max(Math.abs(dx),Math.abs(dy))>9)taskSwipe.axis=Math.abs(dx)>Math.abs(dy)*1.5?'x':'y';
  if(taskSwipe.axis!=='x')return;
  e.preventDefault();taskSwipe.dx=dx;taskSwipe.track.scrollLeft=taskSwipe.left-dx;
},{passive:false});
function finishTaskSwipe(cancelled=false){
  if(!taskSwipe)return;
  const swipe=taskSwipe;taskSwipe=null;
  if(swipe.axis==='x'){const distance=Math.abs(swipe.dx),flick=distance>12&&distance/Math.max(1,performance.now()-swipe.started)>.4,step=!cancelled&&(distance>Math.min(36,swipe.track.clientWidth*.1)||flick)?-Math.sign(swipe.dx):0;scrollToPage(Math.max(0,Math.min(swipe.track.children.length-1,swipe.level+step)))}
  else swipe.track.classList.remove('is-swiping');
}
root.addEventListener('touchend',e=>{const touch=taskSwipe&&[...e.changedTouches].find(touch=>touch.identifier===taskSwipe.id);if(touch)taskSwipe.dx=touch.clientX-taskSwipe.x;finishTaskSwipe()},{passive:true});
root.addEventListener('touchcancel',()=>finishTaskSwipe(true),{passive:true});
function settleCarousel(){
  if(!carouselTarget||taskSwipe)return;
  const {track,left}=carouselTarget;
  if(Math.abs(track.scrollLeft-left)>1)track.scrollTo({left,behavior:'instant'});
  carouselTarget=null;track.classList.remove('is-swiping');
}
function scrollToPage(index,behavior='smooth'){
  const track=root.querySelector('.levels-track'),page=track?.children[index];if(!track||!page)return;
  const left=Math.max(0,Math.min(track.scrollWidth-track.clientWidth,page.offsetLeft+(page.offsetWidth-track.clientWidth)/2));
  clearTimeout(carouselSettleTimer);track.classList.add('is-swiping');carouselTarget={track,left};
  const instant=behavior==='instant'||matchMedia('(prefers-reduced-motion: reduce)').matches;
  track.scrollTo({left,behavior:instant?'instant':behavior});
  if(instant)settleCarousel();else carouselSettleTimer=setTimeout(settleCarousel,140);
}
root.addEventListener('scroll',e=>{if(e.target===carouselTarget?.track){clearTimeout(carouselSettleTimer);carouselSettleTimer=setTimeout(settleCarousel,140)}},true);
window.addEventListener('resize',()=>{alignCarouselPages();updateTrackHeight();if(menuId)positionTaskMenu()});
window.addEventListener('popstate',e=>{if(e.state?.todoNavigation!==navigationId)return;clearTimeout(scrollHistoryTimer);const last=Math.max(0,(root.querySelector('.levels-track')?.children.length||1)-1);historyLevel=Math.min(e.state.todoLevel||0,last);restoringHistory=true;scrollToPage(historyLevel);setTimeout(()=>{restoringHistory=false},700)});
// Les tâches et les pages conservent leur élément DOM entre deux rendus.
function nodeKey(node){if(node.nodeType!==Node.ELEMENT_NODE)return null;if(node.classList.contains('heading'))return 'heading';if(node.classList.contains('quick-add'))return 'quick-add';if(node.classList.contains('task'))return `task:${node.dataset.id}`;if(node.classList.contains('level-page'))return `page:${node.dataset.level}`;return null}
function reconcile(current,next){
  if(current.nodeType!==next.nodeType||current.nodeName!==next.nodeName){current.replaceWith(next);return}
  if(current.nodeType===Node.TEXT_NODE){if(current.nodeValue!==next.nodeValue)current.nodeValue=next.nodeValue;return}
  if(current.classList.contains('heading'))return;
  const layoutStyle=current.matches('.levels-track,.level-page');
  for(const attr of [...current.attributes])if(!next.hasAttribute(attr.name)&&!(layoutStyle&&attr.name==='style'))current.removeAttribute(attr.name);
  for(const attr of [...next.attributes])if(current.getAttribute(attr.name)!==attr.value)current.setAttribute(attr.name,attr.value);
  if(current instanceof HTMLInputElement){if(current.type==='checkbox')current.checked=next.checked;else if(document.activeElement!==current&&current.value!==next.value)current.value=next.value}
  const keyed=new Map([...current.childNodes].map(node=>[nodeKey(node),node]).filter(([key])=>key));
  let cursor=current.firstChild;
  for(const desired of [...next.childNodes]){const key=nodeKey(desired);let match=key?keyed.get(key):cursor;if(match&&(!key&&nodeKey(match)||match.nodeType!==desired.nodeType||match.nodeName!==desired.nodeName))match=null;if(!match){current.insertBefore(desired,cursor);continue}if(match!==cursor)current.insertBefore(match,cursor);reconcile(match,desired);cursor=match.nextSibling}
  while(cursor){const nextNode=cursor.nextSibling;cursor.remove();cursor=nextNode}
}
function render(){const scrollY=window.scrollY;document.getElementById('count').textContent=`${tasks.filter(t=>!t.completed_at).length} en cours`;document.getElementById('edit-order').setAttribute('aria-pressed',String(editingOrder));document.getElementById('group-category').setAttribute('aria-pressed',String(sort==='category'));root.className='tree mode-3';
{const oldTrack=root.querySelector('.levels-track');if(oldTrack){const center=oldTrack.getBoundingClientRect().left+oldTrack.clientWidth/2;let distance=Infinity;[...oldTrack.children].forEach((page,i)=>{const rect=page.getBoundingClientRect(),d=Math.abs(rect.left+rect.width/2-center);if(d<distance){distance=d;carouselLevel=i}})}const validPath=[];for(const id of carouselPath){const parentId=validPath.length?validPath[validPath.length-1]:null;if(tasksById.get(id)?.parent_id!==parentId)break;validPath.push(id)}carouselPath=validPath;const pages=[null,...carouselPath];carouselLevel=Math.min(carouselLevel,pages.length-1);const level=carouselLevel;if(historyLevel>=pages.length){historyLevel=pages.length-1;history.replaceState({...history.state,todoNavigation:navigationId,todoLevel:historyLevel,todoPrevious:null},'')}const desired=root.cloneNode(false);desired.innerHTML=`<div class="levels-track">${pages.map((id,i)=>{const parent=tasksById.get(id);return `<section class="level-page" data-level="${i}">${i===0?'<div class="heading"></div>':''}${parent?`<div class="carousel-page-title">${esc(parent.title)}</div>`:''}<form class="quick-add" data-parent-id="${esc(id||'')}"><input name="title" maxlength="160" required placeholder="Ajouter une t\u00e2che..." aria-label="Ajouter une t\u00e2che"><button type="submit" aria-label="Ajouter la t\u00e2che">+</button></form>${grouped(childrenByParent.get(id)||[],t=>`<div class="task ${t.completed_at?'done':''} ${selected===t.id?'open':''} ${carouselPath.includes(t.id)?'in-path':''}" data-id="${esc(t.id)}">${row(t)}${selected===t.id?`<div class="details-description">${desc(t)}</div>`:''}</div>`)||'<p class="desc none">Aucune tâche dans cette liste.</p>'}</section>`}).join('')}</div>${pages.length>1?dots(pages.length,level):''}`;if(!root.contains(heading))desired.querySelector('.heading').replaceWith(heading);reconcile(root,desired);alignCarouselPages();observePages();scrollToPage(level,'instant');updateTrackHeight();requestAnimationFrame(()=>{scrollToPage(level,'instant');updateTrackHeight()})}
if(window.scrollY!==scrollY)window.scrollTo({top:scrollY,behavior:'instant'});
}
// Reconstruire les index ensemble après chaque chargement de la liste.
function refreshSafe(){const active=document.activeElement;return !document.hidden&&!mutationCount&&!taskSwipe&&!carouselTarget&&modal.classList.contains('hidden')&&activityOverlay.classList.contains('hidden')&&categoryOverlay.classList.contains('hidden')&&searchOverlay.classList.contains('hidden')&&shareOverlay.classList.contains('hidden')&&!menuId&&!(root.contains(active)&&active.matches('input,textarea,select,[contenteditable]'))&&!document.querySelector(':active.task-row')}
async function refresh(background=false){if(background&&!refreshSafe())return;const seq=++refreshSeq,version=interactionVersion;const incoming=await api('/api/tasks');if(seq!==refreshSeq||mutationCount||(background&&(!refreshSafe()||version!==interactionVersion)))return;if(background&&JSON.stringify(incoming)===JSON.stringify(tasks)){updateTaskAges();return}tasks=incoming;tasksById=new Map();childrenByParent=new Map();for(const task of tasks){tasksById.set(task.id,task);const siblings=childrenByParent.get(task.parent_id)||[];siblings.push(task);childrenByParent.set(task.parent_id,siblings)}rebuildSearchIndex();if(selected&&!tasksById.has(selected))selected=null;render()}
async function initialize(){await refresh();const token=sessionStorage.getItem('todoPendingShare');if(token)await showSharePreview(token)}
window.addEventListener('hashchange',()=>{const token=captureShareLink();if(token)showSharePreview(token)});
function backgroundRefresh(){refresh(true).catch(()=>{})}
document.addEventListener('visibilitychange',()=>{if(!document.hidden)backgroundRefresh()});window.addEventListener('focus',backgroundRefresh);setInterval(backgroundRefresh,30000);
document.addEventListener('pointerdown',()=>interactionVersion++,true);document.addEventListener('keydown',()=>interactionVersion++,true);
function showForm(t=null,p=null){form.querySelector("#category-suggestions").innerHTML=[...new Set(tasks.map(task=>task.category).filter(Boolean))].sort((a,b)=>a.localeCompare(b)).map(category=>`<option value="${esc(category)}"></option>`).join("");form.reset();form.elements.title.setCustomValidity('');form.elements.id.value=t?.id||'';form.elements.parent_id.value=p||t?.parent_id||'';form.elements.title.value=t?.title||'';form.elements.description.value=t?.description||'';form.elements.category.value=t?.category||'';form.elements.position.value=t?Math.max(0,t.sort_order):'';form.elements.recurrence.value=t?.recurrence||'none';document.getElementById('modal-title').textContent=t?'Modifier la tâche':p?'Nouvelle sous-tâche':'Nouvelle tâche';modal.classList.remove('hidden');form.elements.title.focus()}
document.getElementById('new-task').onclick=()=>showForm();document.getElementById('cancel').onclick=()=>modal.classList.add('hidden');modal.onclick=e=>{if(e.target===modal)modal.classList.add('hidden')};document.addEventListener('keydown',e=>{if(e.key==='Escape'){modal.classList.add('hidden');closeActivity();closeCategory();closeSearch();if(!shareOverlay.classList.contains('hidden'))closeShare();closeTaskMenu(true);render()}});
form.onsubmit=async e=>{e.preventDefault();const id=form.elements.id.value,d={title:form.elements.title.value.trim(),description:form.elements.description.value.trim(),parent_id:form.elements.parent_id.value||null,category:form.elements.category.value.trim().toLowerCase(),recurrence:form.elements.recurrence.value,position:form.elements.position.value===''?null:Math.max(0,Number(form.elements.position.value))};if(!d.title){form.elements.title.setCustomValidity('Saisis un titre contenant au moins un caractère.');form.elements.title.reportValidity();return}form.elements.title.setCustomValidity('');try{await api(id?`/api/tasks/${id}`:'/api/tasks',{method:id?'PATCH':'POST',body:JSON.stringify(d)});if(!id&&d.parent_id===selected){const path=[];let parent=tasksById.get(d.parent_id);while(parent){path.unshift(parent.id);parent=tasksById.get(parent.parent_id)}carouselPath=path}modal.classList.add('hidden');await refresh()}catch(x){notify(x.message)}};
form.elements.title.addEventListener('input',()=>form.elements.title.setCustomValidity(''));
root.addEventListener('input',e=>{if(e.target.closest('.quick-add'))e.target.setCustomValidity('')});
root.addEventListener('submit',async e=>{const quick=e.target.closest('.quick-add');if(!quick)return;e.preventDefault();if(quick.dataset.pending)return;const input=quick.elements.title,title=input.value.trim();if(!title){input.setCustomValidity('Saisis un titre contenant au moins un caractère.');input.reportValidity();return}input.setCustomValidity('');quick.dataset.pending='true';const button=quick.querySelector('button');button.disabled=true;try{await api('/api/tasks',{method:'POST',body:JSON.stringify({title,parent_id:quick.dataset.parentId||null,recurrence:'none'})});if(input.value.trim()===title)input.value='';await refresh();if(quick.isConnected)input.focus({preventScroll:true})}catch(error){notify(error.message)}finally{delete quick.dataset.pending;button.disabled=false}});
async function taskAction(e){const dot=e.target.closest('[data-page-dot]');if(dot){closeTaskMenu();scrollToPage(Number(dot.dataset.pageDot));return}const a=e.target.closest('[data-act]');if(!a||a.matches('input'))return;const id=a.closest('.task')?.dataset.id||a.closest('.task-menu')?.dataset.taskId,node=id?root.querySelector(`[data-id="${CSS.escape(id)}"]`):null,t=tasksById.get(id);if(!t)return;const act=a.dataset.act;if(act==='position-minus'||act==='position-plus'){const input=node.querySelector('[data-act=position]');input.value=Math.max(0,(Number(input.value)||0)+(act==='position-minus'?-1:1));input.defaultValue=input.value;input.dispatchEvent(new Event('change',{bubbles:true}));return}if(act==='menu'){toggleTaskMenu(id);return}closeTaskMenu();if(act==='expand'){const closing=selected===id;selected=closing?null:id;const level=Number(node.closest('[data-level]').dataset.level);carouselPath=carouselPath.slice(0,level);if(!closing&&childrenByParent.has(id))carouselPath.push(id);render();return}if(act==='share'){showShareLink(t);return}if(act==='category'){showCategory(t);return}if(act==='add-child'){showForm(null,id);return}if(act==='edit'){showForm(t);return}if(act==='delete'&&!confirm('Supprimer cette tâche et ses sous-tâches ?')){render();return}try{if(act==='delete')await api(`/api/tasks/${id}`,{method:'DELETE'});if(act==='up'||act==='down')await api(`/api/tasks/${id}/move`,{method:'POST',body:JSON.stringify({direction:act})});await refresh()}catch(x){notify(x.message)}}
root.onclick=taskAction;taskMenu.onclick=taskAction;
root.addEventListener('scroll',e=>{const track=e.target;if(!track.matches('.levels-track'))return;closeTaskMenu();const center=track.getBoundingClientRect().left+track.clientWidth/2;let index=0,distance=Infinity;[...track.children].forEach((page,i)=>{const r=page.getBoundingClientRect(),d=Math.abs(r.left+r.width/2-center);if(d<distance){index=i;distance=d}});carouselLevel=index;updateTrackHeight();root.querySelectorAll('[data-page-dot]').forEach((dot,i)=>{if(i===index)dot.setAttribute('aria-current','page');else dot.removeAttribute('aria-current')});clearTimeout(scrollHistoryTimer);if(!restoringHistory)scrollHistoryTimer=setTimeout(()=>{if(index===historyLevel)return;if(index>historyLevel)history.pushState({...history.state,todoNavigation:navigationId,todoLevel:index,todoPrevious:historyLevel},'');else history.replaceState({...history.state,todoNavigation:navigationId,todoLevel:index,todoPrevious:null},'');historyLevel=index},160)},true);
root.onchange=async e=>{const act=e.target.dataset.act,id=e.target.closest('.task')?.dataset.id;if(!id)return;try{if(act==='check')await api(`/api/tasks/${id}/check`,{method:'POST',body:JSON.stringify({done:e.target.checked})});else if(act==='position')await api(`/api/tasks/${id}/position`,{method:'POST',body:JSON.stringify({position:Math.max(0,Number(e.target.value)||0)})});else return;await refresh()}catch(x){notify(x.message);render()}};
controls.addEventListener('click',e=>{if(e.target.closest('#edit-order'))editingOrder=!editingOrder;else if(e.target.closest('#group-category'))sort=sort==='category'?'custom':'category';else return;closeTaskMenu();render()});
root.ondragstart=e=>{const n=e.target.closest('.task');if(n)e.dataTransfer.setData('text/plain',n.dataset.id)};root.ondragover=e=>{if(e.target.closest('.task-row'))e.preventDefault()};root.ondrop=async e=>{e.preventDefault();if(sort!=='custom')return;const id=e.target.closest('.task')?.dataset.id,drag=e.dataTransfer.getData('text/plain'),a=tasksById.get(drag),b=tasksById.get(id);if(!a||!b||a.parent_id!==b.parent_id||a.id===b.id)return;const siblings=[...(childrenByParent.get(a.parent_id)||[])].sort((x,y)=>x.sort_order-y.sort_order);try{await api(`/api/tasks/${drag}/position`,{method:'POST',body:JSON.stringify({position:siblings.findIndex(t=>t.id===b.id)})});await refresh()}catch(x){notify(x.message)}};
initialize().catch(x=>notify(x.message));
})();
