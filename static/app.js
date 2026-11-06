(() => {
  const root = document.getElementById('tree');
  if (!root) return;
  const modal = document.getElementById('modal');
  const form = document.getElementById('task-form');
  const expanded = new Set();
  let menuOpenId = null;
  let tasks = [];
  const escape = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const csrfFetch = async (url, options={}) => {
    const response = await fetch(url, {credentials:'same-origin', headers:{'Content-Type':'application/json','X-Todo-Request':'1', ...(options.headers||{})}, ...options});
    if(response.status === 401){window.location.href='/auth/login'; throw new Error('Session expirée');}
    if(!response.ok){const data=await response.json().catch(()=>({}));throw new Error(data.error||'Une erreur est survenue');}
    if(response.status === 204)return null;
    return response.json();
  };
  let toastTimer;
  const notify = txt => {let t=document.querySelector('.toast'); if(!t){t=document.createElement('div');t.className='toast';document.body.append(t);}t.textContent=txt;clearTimeout(toastTimer);toastTimer=setTimeout(()=>t.remove(),3500);};
  function render(){
    const byParent = new Map();
    for(const t of tasks){ const p=t.parent_id??'';if(!byParent.has(p))byParent.set(p,[]);byParent.get(p).push(t);}
    const renderLevel=(parent,depth=0)=> (byParent.get(parent)||[]).map(t=>{
      const open=expanded.has(t.id), done=!!t.completed_at;
      const menuOpen=menuOpenId===t.id;
      return `<div class="task ${open?'open':''} ${done?'done':''} ${menuOpen?'menu-open':''}" data-id="${escape(t.id)}">
        <div class="task-row" draggable="true" data-drag-id="${escape(t.id)}"><input class="check" type="checkbox" data-act="check" aria-label="Valider ${escape(t.title)}" ${done?'checked':''}>
        <button class="task-name" data-act="expand" aria-expanded="${open}" aria-label="${open?'Replier':'Déplier'} ${escape(t.title)}">${escape(t.title)}</button>
        <button class="iconbtn" data-act="add-child" title="Ajouter une sous-tâche" aria-label="Ajouter une sous-tâche à ${escape(t.title)}">+</button>
        <div class="menu-wrap">
          <button class="iconbtn menu-toggle" data-act="menu" aria-haspopup="true" aria-expanded="${menuOpen}" aria-label="Actions pour ${escape(t.title)}" title="Actions">⋯</button>
          ${menuOpen?`<div class="task-menu" role="group" aria-label="Actions pour ${escape(t.title)}">
            <button data-act="edit">Modifier</button>
            <button data-act="up">↑ Monter</button>
            <button data-act="down">↓ Descendre</button>
            <button class="danger" data-act="delete">Supprimer</button>
          </div>`:''}
        </div>
        <button class="iconbtn expand-toggle" data-act="expand" aria-label="${open?'Masquer':'Afficher'} les détails et sous-tâches de ${escape(t.title)}" aria-expanded="${open}"><span class="twisty">▶</span></button></div>
        ${open?`<div class="details"><div class="desc ${t.description?'':'none'}">${escape(t.description||'Aucune description')}</div></div>
        ${depth<30?`<div class="children">${renderLevel(t.id,depth+1)}</div>`:''}`:''}</div>`;
    }).join('');
    root.innerHTML=renderLevel('') || '<div class="empty"><div class="big">☑</div>Aucune tâche pour le moment.<br>Ajoutez votre première tâche.</div>';
    document.getElementById('count').textContent=`${tasks.filter(t=>!t.completed_at).length} en cours`;
  }
  async function refresh(){tasks=await csrfFetch('/api/tasks');render();}
  function showForm(t=null,parentId=null){
    form.reset();form.elements.id.value=t?.id||'';form.elements.parent_id.value=parentId||t?.parent_id||'';
    form.elements.title.value=t?.title||'';form.elements.description.value=t?.description||'';
    document.getElementById('modal-title').textContent=t?'Modifier la tâche':parentId?'Nouvelle sous-tâche':'Nouvelle tâche';
    modal.classList.remove('hidden');form.elements.title.focus();
  }
  document.getElementById('new-task').addEventListener('click',()=>showForm());
  document.getElementById('cancel').addEventListener('click',()=>modal.classList.add('hidden'));
  modal.addEventListener('click',e=>{if(e.target===modal)modal.classList.add('hidden');});
  document.addEventListener('keydown',e=>{if(e.key==='Escape'){modal.classList.add('hidden');if(menuOpenId!==null){menuOpenId=null;render();}}});
  form.addEventListener('submit',async e=>{
    e.preventDefault(); const id=form.elements.id.value;
    const data={title:form.elements.title.value.trim(),description:form.elements.description.value.trim(),parent_id:form.elements.parent_id.value||null};
    if(!data.title)return;
    try{await csrfFetch(id?`/api/tasks/${encodeURIComponent(id)}`:'/api/tasks',{method:id?'PATCH':'POST',body:JSON.stringify(data)});modal.classList.add('hidden');await refresh();}
    catch(err){notify(err.message);}
  });
  root.addEventListener('click',async e=>{
    const target=e.target.closest('[data-act]');if(!target||target.dataset.act==='check')return;
    const node=target.closest('.task');if(!node)return;
    const id=node.dataset.id, task=tasks.find(t=>t.id===id);if(!task)return;
    const act=target.dataset.act;
    if(act==='menu'){menuOpenId=menuOpenId===id?null:id;render();return;}
    menuOpenId=null;
    if(act==='expand'){expanded.has(id)?expanded.delete(id):expanded.add(id);render();return;}
    if(act==='add-child'){expanded.add(id);render();showForm(null,id);return;}
    if(act==='edit'){render();showForm(task);return;}
    if(act==='delete'&&!confirm('Supprimer cette tâche et toutes ses sous-tâches ?')){render();return;}
    try{
      if(act==='delete')await csrfFetch(`/api/tasks/${encodeURIComponent(id)}`,{method:'DELETE'});
      if(act==='up'||act==='down')await csrfFetch(`/api/tasks/${encodeURIComponent(id)}/move`,{method:'POST',body:JSON.stringify({direction:act})});
      await refresh();
    }catch(err){notify(err.message);}
  });
  document.addEventListener('click',e=>{
    if(menuOpenId!==null && !e.target.closest('.menu-wrap')){menuOpenId=null;render();}
  });
  root.addEventListener('change',async e=>{
    if(!e.target.matches('[data-act="check"]'))return;
    const id=e.target.closest('.task').dataset.id;
    try{await csrfFetch(`/api/tasks/${encodeURIComponent(id)}/check`,{method:'POST',body:JSON.stringify({done:e.target.checked})});await refresh();}
    catch(err){notify(err.message);await refresh();}
  });
  root.addEventListener('dragstart',e=>{
    const row=e.target.closest('.task-row');
    if(!row)return;
    const task=row.closest('.task');
    if(!task)return;
    e.dataTransfer.effectAllowed='move';
    e.dataTransfer.setData('text/plain',task.dataset.id);
    task.classList.add('dragging');
  });
  root.addEventListener('dragend',e=>{
    const task=e.target.closest('.task');
    if(task)task.classList.remove('dragging');
    root.querySelectorAll('.drag-over').forEach(el=>el.classList.remove('drag-over'));
  });
  root.addEventListener('dragover',e=>{
    const row=e.target.closest('.task-row');
    if(!row)return;
    const target=row.closest('.task');
    const dragged=tasks.find(t=>t.id===e.dataTransfer.getData('text/plain'));
    const targetTask=tasks.find(t=>t.id===target?.dataset.id);
    if(!dragged||!targetTask||dragged.id===targetTask.id||dragged.parent_id!==targetTask.parent_id)return;
    e.preventDefault();
    e.dataTransfer.dropEffect='move';
    root.querySelectorAll('.drag-over').forEach(el=>el.classList.remove('drag-over'));
    row.classList.add('drag-over');
  });
  root.addEventListener('dragleave',e=>{
    const row=e.target.closest('.task-row');
    if(row&&!row.contains(e.relatedTarget))row.classList.remove('drag-over');
  });
  root.addEventListener('drop',async e=>{
    const row=e.target.closest('.task-row');
    if(!row)return;
    e.preventDefault();
    const targetId=row.closest('.task')?.dataset.id;
    const draggedId=e.dataTransfer.getData('text/plain');
    const dragged=tasks.find(t=>t.id===draggedId);
    const target=tasks.find(t=>t.id===targetId);
    if(!dragged||!target||draggedId===targetId||dragged.parent_id!==target.parent_id)return;
    const siblings=tasks.filter(t=>t.parent_id===dragged.parent_id);
    const from=siblings.findIndex(t=>t.id===draggedId), to=siblings.findIndex(t=>t.id===targetId);
    if(from<0||to<0||from===to)return;
    try{
      const direction=from<to?'down':'up';
      for(let i=0;i<Math.abs(to-from);i++){
        await csrfFetch(`/api/tasks/${encodeURIComponent(draggedId)}/move`,{method:'POST',body:JSON.stringify({direction})});
      }
      await refresh();
    }catch(err){notify(err.message);await refresh();}
    root.querySelectorAll('.drag-over').forEach(el=>el.classList.remove('drag-over'));
  });
  refresh().catch(err=>notify(err.message));
})();
