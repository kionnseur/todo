const { test, expect } = require('@playwright/test');

function task(id, title, parentId = null, category = '') {
  return { id, title, parent_id: parentId, description: '', category, category_color: '0', recurrence: 'none', sort_order: 0, completed_at: null, next_reset_at: null, created_at: 1 };
}

function mockApi(context, state) {
  return context.route('**/api/**', async route => {
    const request = route.request();
    const url = new URL(request.url());
    const json = (status, value) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(value) });
    const shareCreate = url.pathname.match(/^\/api\/tasks\/([^/]+)\/share$/);
    if (shareCreate && request.method() === 'POST') {
      state.snapshot = state.tasks.filter(item => item.id === shareCreate[1] || item.parent_id === shareCreate[1]).map(item => ({ ...item, parent_id: item.id === shareCreate[1] ? null : item.parent_id }));
      state.lastShareOptions = request.postDataJSON();
      return json(200, { token: 's'.repeat(43), expires_at: state.lastShareOptions.duration_seconds ? Math.floor(Date.now() / 1000) + state.lastShareOptions.duration_seconds : null });
    }
    if (url.pathname === `/api/shares/${'s'.repeat(43)}`) {
      if (request.method() === 'GET') return json(200, { tasks: state.snapshot, already_imported: Boolean(state.imports) });
      if (request.method() === 'POST') { state.imports = (state.imports || 0) + 1; state.lastDestination = request.postDataJSON(); return json(200, { id: 'copy' }); }
    }
    if (url.pathname === '/api/tasks' && request.method() === 'GET') {
      return json(200, state.tasks.map(item => ({ ...item, category_color: String(item.category ? 4 : 0) })));
    }
    if (url.pathname === '/api/tasks' && request.method() === 'POST') {
      const data = request.postDataJSON();
        const item = { ...task(`new-${state.tasks.length}`, data.title, data.parent_id, data.category), ...data };
      state.tasks.push(item);
      return json(201, { id: item.id });
    }
    const category = url.pathname.match(/^\/api\/tasks\/([^/]+)\/category$/);
    if (category && request.method() === 'PATCH') {
      const item = state.tasks.find(item => item.id === category[1]);
      if (!item) return json(404, { error: 'Tâche introuvable' });
      item.category = request.postDataJSON().category;
      return route.fulfill({ status: 204 });
    }
    if (url.pathname === '/api/activity') return json(200, state.events || []);
    if (url.pathname === '/api/activity/upcoming') return json(200, { events: state.upcoming || [], timezone: 'Europe/Paris' });
    if (url.pathname === '/api/shares' && request.method() === 'GET') return state.failShares ? json(500, { error: 'Liste indisponible' }) : json(200, state.shares || []);
    const revoke = url.pathname.match(/^\/api\/shares\/manage\/([^/]+)$/);
    if (revoke && request.method() === 'DELETE') {
      state.revoked = revoke[1]; state.shares = state.shares.filter(link => link.id !== revoke[1]);
      return route.fulfill({ status: 204 });
    }
    return json(404, { error: 'Route non prévue par le test' });
  });
}

test('créer une sous-tâche et revenir avec les boutons et le navigateur', async ({ page, context }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const state = { tasks: [task('parent', 'Parent')], colors: {} };
  await mockApi(context, state);
  await page.goto('/');
  await page.getByRole('button', { name: 'Parent', exact: true }).click();
  await page.locator('[data-id="parent"]').getByRole('button', { name: 'Ajouter une sous-tâche' }).click();
  await page.locator('#task-title').fill('Enfant');
  await page.getByRole('button', { name: 'Enregistrer' }).click();
  await expect(page.locator('.level-page')).toHaveCount(2);
  await expect(page.getByRole('button', { name: 'Enfant', exact: true })).toBeAttached();
  await page.getByRole('button', { name: 'Page 2' }).click();
  await expect(page.getByRole('button', { name: 'Page 2' })).toHaveAttribute('aria-current', 'page');
  await expect.poll(() => page.evaluate(() => history.state.todoLevel)).toBe(1);
  await page.goBack();
  await expect(page.getByRole('button', { name: 'Page 1' })).toHaveAttribute('aria-current', 'page');
});

test('synchroniser les couleurs et actualiser sans interrompre une saisie', async ({ browser }) => {
  const state = { tasks: [task('first', 'Première', null, 'maison')], colors: {} };
  const first = await browser.newContext();
  const second = await browser.newContext();
  try {
    await mockApi(first, state); await mockApi(second, state);
    const a = await first.newPage(); const b = await second.newPage();
    await a.goto('http://127.0.0.1:4187/'); await b.goto('http://127.0.0.1:4187/');
    await expect(a.locator('[data-id="first"] .category-dot')).toHaveClass(/category-color-4/);
    await expect(b.locator('[data-id="first"] .category-dot')).toHaveClass(/category-color-4/);
    await b.getByRole('button', { name: '+ Nouvelle tâche' }).click();
    await b.locator('#task-title').fill('Saisie en cours');
    await expect(b.locator('#category-suggestions option')).toHaveAttribute('value', 'maison');
    await b.locator('#task-form [name=category]').fill('MAISON');
    state.tasks.push(task('remote', 'Ajout distant'));
    await b.evaluate(() => window.dispatchEvent(new Event('focus')));
    await expect(b.locator('#task-title')).toHaveValue('Saisie en cours');
    await expect(b.locator('[data-id="remote"]')).toHaveCount(0);
    await b.getByRole('button', { name: 'Enregistrer' }).click();
    expect(state.tasks.find(item => item.title === 'Saisie en cours').category).toBe('maison');
    await b.evaluate(() => window.dispatchEvent(new Event('focus')));
    await expect(b.locator('[data-id="remote"]')).toHaveCount(1);
  } finally { await first.close(); await second.close(); }
});

test('actualiser périodiquement une liste laissée ouverte', async ({ page, context }) => {
  const state = { tasks: [task('first', 'Première')], colors: {} };
  await mockApi(context, state);
  await page.clock.install();
  await page.goto('/');
  await expect(page.locator('[data-id="first"]')).toHaveCount(1);
  state.tasks.push(task('remote', 'Ajout distant'));
  await page.clock.fastForward(30001);
  await expect(page.locator('[data-id="remote"]')).toHaveCount(1);
});

test('conserver le défilement vertical à la sélection et désélection', async ({ page, context }) => {
  await page.setViewportSize({ width: 390, height: 600 });
  const state = { tasks: Array.from({ length: 40 }, (_, i) => task(`task-${i}`, `Tâche ${i}`)), colors: {} };
  await mockApi(context, state);
  await page.goto('/');
  await expect(page.locator('[data-id="task-20"]')).toBeAttached();
  await page.locator('[data-id="task-20"]').scrollIntoViewIfNeeded();
  const before = await page.evaluate(() => window.scrollY);
  for (let i = 0; i < 4; i++) {
    await page.locator('[data-id="task-20"] .task-name').click();
    await page.waitForTimeout(50);
    expect(Math.abs((await page.evaluate(() => window.scrollY)) - before)).toBeLessThanOrEqual(1);
  }
});

test('conserver les éléments existants pendant la sélection et une actualisation', async ({ page, context }) => {
  const state = { tasks: [task('parent', 'Parent'), task('child', 'Enfant', 'parent'), task('other', 'Autre')], colors: {} };
  await mockApi(context, state);
  await page.goto('/');
  await expect(page.locator('[data-id="parent"]')).toBeAttached();
  await page.evaluate(() => { window.savedParent = document.querySelector('[data-id="parent"]'); window.savedButton = window.savedParent.querySelector('.task-name'); window.savedTrack = document.querySelector('.levels-track'); });
  await page.locator('[data-id="parent"] .task-name').click();
  await expect(page.locator('.level-page')).toHaveCount(2);
  expect(await page.evaluate(() => window.savedParent === document.querySelector('[data-id="parent"]') && window.savedButton === document.activeElement && window.savedTrack === document.querySelector('.levels-track'))).toBe(true);
  state.tasks.push(task('remote', 'Ajout distant'));
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect(page.locator('[data-id="remote"]')).toBeAttached();
  expect(await page.evaluate(() => window.savedParent === document.querySelector('[data-id="parent"]') && window.savedTrack === document.querySelector('.levels-track'))).toBe(true);
});

test('ajouter rapidement des tâches à la racine et dans une liste enfant', async ({ page, context }) => {
  const state = { tasks: [task('parent', 'Parent'), task('child', 'Enfant', 'parent')], colors: {} };
  await mockApi(context, state);
  await page.goto('/');
  const rootInput = page.locator('[data-level="0"] .quick-add input');
  expect(await rootInput.evaluate(input => Boolean(input.closest('.quick-add').compareDocumentPosition(document.querySelector('[data-id="parent"]')) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
  await rootInput.fill('  Nouvelle tâche  ');
  await rootInput.press('Enter');
  await expect(page.getByRole('button', { name: 'Nouvelle tâche', exact: true })).toBeVisible();
  expect(state.tasks.find(item => item.title === 'Nouvelle tâche').parent_id).toBeNull();
  await expect(rootInput).toHaveValue('');
  await rootInput.fill('   ');
  await rootInput.press('Enter');
  expect(await rootInput.evaluate(input => input.validationMessage)).not.toBe('');
  await rootInput.fill('');
  await page.getByRole('button', { name: 'Parent', exact: true }).click();
  const childInput = page.locator('[data-level="1"] .quick-add input');
  await childInput.fill('Sous-tâche rapide');
  await childInput.press('Enter');
  await expect(page.getByRole('button', { name: 'Sous-tâche rapide', exact: true })).toBeAttached();
  expect(state.tasks.find(item => item.title === 'Sous-tâche rapide').parent_id).toBe('parent');
});

test('remonter en haut avec le bouton flottant', async ({ page, context }) => {
  await page.setViewportSize({ width: 390, height: 600 });
  const state = { tasks: Array.from({ length: 40 }, (_, i) => task(`task-${i}`, `Tâche ${i}`)), colors: {} };
  await mockApi(context, state);
  await page.goto('/');
  const button = page.getByRole('button', { name: 'Remonter en haut de la page' });
  await expect(button).toBeHidden();
  await page.evaluate(() => window.scrollTo(0, 800));
  await expect(button).toBeVisible();
  await button.click();
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await expect(button).toBeHidden();
});

test('adapter la hauteur du carrousel à la liste visible', async ({ page, context }) => {
  await page.setViewportSize({ width: 390, height: 600 });
  const state = { tasks: [task('parent', 'Parent'), task('child', 'Enfant', 'parent'), ...Array.from({ length: 40 }, (_, i) => task(`task-${i}`, `Tâche ${i}`))], colors: {} };
  await mockApi(context, state);
  await page.goto('/');
  await page.getByRole('button', { name: 'Parent', exact: true }).click();
  const track = page.locator('.levels-track');
  const rootHeight = await track.evaluate(element => element.clientHeight);
  await page.getByRole('button', { name: 'Page 2' }).click();
  await expect(page.getByRole('button', { name: 'Page 2' })).toHaveAttribute('aria-current', 'page');
  await expect.poll(() => track.evaluate(element => element.clientHeight)).toBeLessThan(rootHeight / 2);
  await page.evaluate(() => window.scrollTo(0, 10000));
  expect(await page.evaluate(() => document.documentElement.scrollHeight - innerHeight - scrollY)).toBeLessThanOrEqual(1);
  await page.getByRole('button', { name: 'Page 1' }).click();
  await expect.poll(() => track.evaluate(element => element.clientHeight)).toBeGreaterThan(rootHeight / 2);
});

test('modifier et supprimer une catégorie avec la pastille', async ({ page, context }) => {
  const state = { tasks: [task('first', 'Première'), task('other', 'Autre', null, 'maison')], colors: {} };
  await mockApi(context, state);
  await page.goto('/');
  await page.getByRole('button', { name: 'Modifier la catégorie de Première' }).click();
  const dialog = page.getByRole('dialog', { name: 'Catégorie' });
  await expect(dialog.locator('datalist option')).toHaveAttribute('value', 'maison');
  await dialog.getByLabel('Nom de la catégorie').fill(' MAISON ');
  await dialog.getByRole('button', { name: 'Enregistrer' }).click();
  await expect(page.locator('[data-id="first"] .category-label')).toHaveText('maison');
  await page.getByRole('button', { name: 'Modifier la catégorie de Première' }).click();
  await dialog.getByLabel('Nom de la catégorie').fill('');
  await dialog.getByRole('button', { name: 'Enregistrer' }).click();
  await expect(page.locator('[data-id="first"] .category-empty')).toHaveCount(1);
});

test('rechercher et ouvrir une tâche imbriquée', async ({ page, context }) => {
  const state = { tasks: [task('a', 'Parent'), task('b', 'Sous-liste', 'a'), task('c', 'Résultat', 'b', 'maison')], colors: {} };
  await mockApi(context, state);
  await page.goto('/');
  await page.getByRole('button', { name: 'Rechercher une tâche' }).click();
  const dialog = page.getByRole('dialog', { name: 'Rechercher une tâche' });
  await dialog.getByLabel('Rechercher', { exact: true }).fill('résultat');
  await expect(dialog.getByText('Parent › Sous-liste')).toBeVisible();
  await dialog.getByRole('button', { name: /Résultat/ }).click();
  await expect(page.locator('[data-level="2"] [data-id="c"]')).toHaveClass(/open/);
  await expect(page.locator('[data-level="2"] [data-id="c"] .task-name')).toBeFocused();
});

test('prévisualiser un partage avant de confirmer l’import', async ({ browser }) => {
  const state = { tasks: [task('parent', 'Parent', null, 'maison'), task('child', 'Enfant', 'parent', 'courses')], colors: {} };
  const first = await browser.newContext(), second = await browser.newContext();
  try {
    await mockApi(first, state); await mockApi(second, state);
    const sender = await first.newPage(); await sender.goto('http://127.0.0.1:4187/');
    await sender.locator('[data-id="parent"]').getByRole('button', { name: 'Actions' }).click();
    await sender.getByRole('menuitem', { name: 'Partager', exact: true }).click();
    await sender.getByRole('button', { name: 'Créer le lien', exact: true }).click();
    const link = await sender.getByLabel('Lien de partage').inputValue();
    const recipient = await second.newPage(); await recipient.goto(link);
    const dialog = recipient.getByRole('dialog', { name: 'Importer les tâches' });
    await expect(dialog.locator('.share-list').getByText('Parent', { exact: true })).toBeVisible(); await expect(dialog.locator('.share-list').getByText('Enfant', { exact: true })).toBeVisible();
    await expect(dialog.getByText('courses')).toBeVisible(); expect(state.imports || 0).toBe(0);
    await dialog.getByLabel('Importer sous').selectOption('child');
    await dialog.getByRole('button', { name: 'Ajouter à mes tâches' }).click();
    await expect(dialog).toBeHidden(); expect(state.imports).toBe(1); expect(state.lastDestination).toEqual({ parent_id: 'child' });
    await recipient.goto(link);
    await dialog.getByLabel('Importer sous').selectOption('__new__');
    await dialog.getByRole('button', { name: 'Ajouter à mes tâches' }).click();
    expect(await dialog.getByLabel('Nom de la nouvelle tâche parente').evaluate(input => input.validity.valid)).toBe(false);
    await dialog.getByLabel('Nom de la nouvelle tâche parente').fill('Projet');
    await dialog.getByRole('button', { name: 'Ajouter à mes tâches' }).click();
    await expect(dialog).toBeHidden(); expect(state.lastDestination).toEqual({ new_parent_title: 'Projet' });
  } finally { await first.close(); await second.close(); }
});

test('ne pas couper les pages voisines plus longues que la page centrale', async ({ page, context }) => {
  await page.setViewportSize({ width: 1520, height: 900 });
  const state = { tasks: [task('parent', 'Parent'), ...Array.from({ length: 24 }, (_, i) => task(`child-${i}`, `Enfant ${i}`, 'parent'))] };
  await mockApi(context, state);
  await page.goto('/');
  await page.locator('[data-id="parent"] .task-name').click();
  const track = page.locator('.levels-track');
  await page.locator('[data-level="1"]').evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished)));
  await expect(page.getByRole('button', { name: 'Page 1', exact: true })).toHaveAttribute('aria-current', 'page');
  await expect.poll(() => track.evaluate(element => {
    const bounds = element.getBoundingClientRect(), neighbor = element.children[1].getBoundingClientRect();
    return neighbor.left < bounds.right && neighbor.bottom <= bounds.bottom + 1;
  })).toBe(true);

  // Une page centrale courte ne doit pas non plus couper sa voisine de gauche.
  state.tasks = [task('parent', 'Parent'), task('child', 'Enfant', 'parent'), ...Array.from({ length: 30 }, (_, i) => task(`root-${i}`, `Racine ${i}`))];
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect(page.locator('[data-id="root-29"]')).toBeAttached();
  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Page 2', exact: true })).toHaveAttribute('aria-current', 'page');
  await expect.poll(() => track.evaluate(element => {
    const bounds = element.getBoundingClientRect(), neighbor = element.children[0].getBoundingClientRect();
    return neighbor.right > bounds.left && neighbor.bottom <= bounds.bottom + 1;
  })).toBe(true);
});

test('aligner chaque sous-page sur sa tâche source, même après actualisation et redimensionnement', async ({ page, context }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const state = { tasks: [
    ...Array.from({ length: 3 }, (_, i) => task(`root-${i}`, `Racine ${i}`)),
    task('parent', 'Parent'),
    ...Array.from({ length: 3 }, (_, i) => task(`child-${i}`, `Enfant ${i}`, 'parent')),
    task('nested', 'Sous-liste', 'parent'), task('leaf', 'Feuille', 'nested'),
  ] };
  await mockApi(context, state);
  await page.goto('/');
  await page.locator('[data-id="parent"] .task-name').click();
  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Page 2', exact: true })).toHaveAttribute('aria-current', 'page');
  await page.locator('[data-id="nested"] .task-name').click();
  const alignmentError = () => page.evaluate(() => Math.max(...['parent', 'nested'].map((id, index) => {
    const source = document.querySelector(`[data-id="${id}"]`).getBoundingClientRect();
    const target = document.querySelector(`[data-level="${index + 1}"]`).getBoundingClientRect();
    return Math.abs(source.top - target.top);
  })));
  await expect.poll(alignmentError).toBeLessThanOrEqual(2);
  const previousTop = await page.locator('[data-level="2"]').evaluate(element => element.getBoundingClientRect().top + scrollY);
  state.tasks.unshift({ ...task('remote', 'Ajout distant'), sort_order: -1 });
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect(page.locator('[data-id="remote"]')).toBeAttached();
  await expect.poll(alignmentError).toBeLessThanOrEqual(2);
  expect(await page.locator('[data-level="2"]').evaluate(element => element.getBoundingClientRect().top + scrollY)).toBeGreaterThan(previousTop);
  for (const width of [1000, 390, 1520]) {
    await page.setViewportSize({ width, height: 900 });
    await expect.poll(alignmentError).toBeLessThanOrEqual(2);
  }
});

test('ouvrir le menu au-delà de la carte et conserver les actions et le clavier', async ({ page, context }) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  await mockApi(context, { tasks: [task('one', 'Une tâche')] });
  await page.goto('/');
  const trigger = page.locator('[data-id="one"] .menu-toggle'), menu = page.getByRole('menu');
  await trigger.click();
  await expect(menu).toBeVisible();
  await expect(trigger).toHaveAttribute('aria-expanded', 'true');
  expect(await menu.evaluate(element => element.parentElement === document.body)).toBe(true);
  expect((await menu.boundingBox()).y + (await menu.boundingBox()).height).toBeGreaterThan(await page.locator('.levels-track').evaluate(element => element.getBoundingClientRect().bottom));
  await expect(menu.getByRole('menuitem', { name: 'Modifier', exact: true })).toBeFocused();
  await page.keyboard.press('End');
  await expect(menu.getByRole('menuitem', { name: 'Supprimer', exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await menu.getByRole('menuitem', { name: 'Modifier', exact: true }).click();
  await expect(page.locator('#task-title')).toHaveValue('Une tâche');
  await expect(menu).toBeHidden();
  await page.keyboard.press('Escape');
  await trigger.click();
  await page.getByRole('heading', { name: 'Mes tâches', exact: true }).click();
  await expect(menu).toBeHidden();
});

test('ouvrir le menu vers le haut en bas d’un petit écran', async ({ page, context }) => {
  await page.setViewportSize({ width: 390, height: 380 });
  await mockApi(context, { tasks: Array.from({ length: 12 }, (_, i) => task(`task-${i}`, `Tâche ${i}`)) });
  await page.goto('/');
  await page.locator('[data-id="task-11"]').scrollIntoViewIfNeeded();
  await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
  await page.locator('[data-id="task-11"] .menu-toggle').click();
  const menu = page.getByRole('menu');
  await expect(menu).toHaveAttribute('data-side', 'top');
  await expect.poll(() => menu.evaluate(element => {
    const rect = element.getBoundingClientRect();
    return rect.top >= 7 && rect.bottom <= innerHeight - 7 && rect.left >= 7 && rect.right <= innerWidth - 7;
  })).toBe(true);
  await menu.getByRole('menuitem', { name: 'Modifier', exact: true }).click();
  await expect(page.locator('#task-title')).toHaveValue('Tâche 11');
});

test('signaler un mois calendaire d’ancienneté, sans indicateur sur les récurrentes ou terminées', async ({ page, context }) => {
  await page.clock.setFixedTime(new Date('2026-02-28T12:00:00+01:00'));
  const created = date => new Date(date).getTime() / 1000;
  const old = { ...task('old', 'Ancienne'), created_at: created('2026-01-15T12:00:00+01:00') };
  const state = { tasks: [old,
    { ...task('end-month', 'Fin de mois'), created_at: created('2026-01-31T12:00:00+01:00') },
    { ...task('recent', 'Récente'), created_at: created('2026-02-01T12:00:00+01:00') },
    { ...task('not-yet', 'Pas encore'), created_at: created('2026-01-31T12:00:01+01:00') },
    { ...task('done', 'Terminée'), created_at: old.created_at, completed_at: created('2026-02-28T11:00:00+01:00') },
    ...['daily', 'weekly', 'monthly', 'quarterly', 'yearly'].map(recurrence => ({ ...task(recurrence, recurrence), created_at: old.created_at, recurrence })),
  ] };
  await mockApi(context, state);
  await page.goto('/');
  await expect(page.locator('.task-age')).toHaveCount(2);
  await expect(page.locator('[data-id="old"] .task-age')).toHaveAttribute('title', /Créée le 15 janvier 2026/);
  await expect(page.locator('[data-id="end-month"] .task-age')).toBeVisible();
  await expect(page.locator('.recurrence-icon')).toHaveCount(5);
  for (const id of ['recent', 'not-yet', 'done', 'daily', 'weekly', 'monthly', 'quarterly', 'yearly']) {
    await expect(page.locator(`[data-id="${id}"] .task-age`)).toHaveCount(0);
  }
});

test('faire apparaître l’ancienneté au seuil même si les données ne changent pas', async ({ page, context }) => {
  await page.clock.install({ time: new Date('2026-02-28T11:59:45+01:00') });
  await mockApi(context, { tasks: [{ ...task('one', 'Une tâche'), created_at: new Date('2026-01-31T12:00:00+01:00').getTime() / 1000 }] });
  await page.goto('/');
  await expect(page.locator('[data-id="one"]')).toBeVisible();
  await expect(page.locator('.task-age')).toHaveCount(0);
  await page.clock.fastForward(30001);
  await expect(page.locator('.task-age')).toHaveCount(1);
});

test('respecter la réduction des animations pour les pages, menus et la navigation', async ({ page, context }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 390, height: 844 });
  await mockApi(context, { tasks: [task('parent', 'Parent'), task('child', 'Enfant', 'parent')] });
  await page.goto('/');
  await page.locator('[data-id="parent"] .task-name').click();
  await page.locator('[data-id="parent"] .menu-toggle').click();
  expect(await page.getByRole('menu').evaluate(element => getComputedStyle(element).animationName)).toBe('none');
  expect(await page.locator('[data-level="1"]').evaluate(element => getComputedStyle(element).animationName)).toBe('none');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Page 2', exact: true })).toHaveAttribute('aria-current', 'page');
});

test('rechercher sans accents, avec les ligatures et plusieurs mots, puis ouvrir au clavier', async ({ page, context }) => {
  const state = { tasks: [
    { ...task('music', 'École de musique', null, 'études'), description: 'Réviser le solfège' },
    task('exact', 'École'), task('parent', 'Appartement'), task('heart', 'Étagère du cœur', 'parent'),
    task('coffee', 'Cafe\u0301 du quartier'),
  ] };
  await mockApi(context, state); await page.goto('/');
  await page.getByRole('button', { name: 'Rechercher une tâche' }).click();
  const dialog = page.getByRole('dialog', { name: 'Rechercher une tâche' }), input = dialog.getByLabel('Rechercher', { exact: true });
  for (const query of ['ecole solfege', 'ÉTUDES ECOLE']) {
    await input.fill(query);
    await expect(dialog.locator('[data-search-id]')).toHaveCount(1);
    await expect(dialog.locator('[data-search-id]')).toHaveAttribute('data-search-id', 'music');
  }
  await input.fill('coeur appartement');
  await expect(dialog.locator('[data-search-id]')).toHaveAttribute('data-search-id', 'heart');
  await input.fill('CAFÉ');
  await expect(dialog.locator('[data-search-id]')).toHaveAttribute('data-search-id', 'coffee');
  await input.fill('ecole');
  await expect(dialog.locator('[data-search-id]').first()).toHaveAttribute('data-search-id', 'exact');
  await input.press('ArrowDown');
  await expect(dialog.locator('[data-search-id="exact"]')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(dialog).toBeHidden();
  await expect(page.locator('[data-id="exact"] .task-name')).toBeFocused();
});

test('consulter les automatismes à venir et les distinguer de l’historique', async ({ page, context }) => {
  const scheduled = new Date('2027-01-04T00:00:00+01:00').getTime() / 1000;
  const state = { tasks: [task('one', 'Une tâche')], upcoming: [
    { task_id: 'reset', kind: 'reset', title: 'Ménage', scheduled_at: scheduled, affected_count: 2 },
    { task_id: 'delete', kind: 'deleted', title: 'Ancien projet', scheduled_at: scheduled + 3600, affected_count: 3 },
  ], events: [{ id: 1, kind: 'deleted', title: 'Déjà supprimée', occurred_at: scheduled - 86400 }] };
  await mockApi(context, state); await page.goto('/');
  await page.getByRole('button', { name: 'Activité automatique' }).click();
  const dialog = page.getByRole('dialog', { name: 'Activité automatique' });
  await expect(dialog.getByRole('tab', { name: 'À venir' })).toHaveAttribute('aria-selected', 'true');
  await expect(dialog.getByText('Ménage', { exact: true })).toBeVisible();
  await expect(dialog.getByText('2 tâches cochées seront décochées.', { exact: true })).toBeVisible();
  await expect(dialog.getByText(/3 tâches seront supprimées, sous-tâches comprises/)).toBeVisible();
  await expect(dialog.locator('time').first()).toHaveText('4 janv. 2027, 00:00');
  await dialog.getByRole('tab', { name: 'Historique' }).click();
  await expect(dialog.getByText('Déjà supprimée', { exact: true })).toBeVisible();
  await expect(dialog.getByText('Ménage', { exact: true })).toHaveCount(0);
  await page.keyboard.press('ArrowLeft');
  await expect(dialog.getByRole('tab', { name: 'À venir' })).toBeFocused();
  await expect(dialog.getByText('Ménage', { exact: true })).toBeVisible();
});

test('ne pas remplacer l’historique par une réponse à venir arrivée en retard', async ({ page, context }) => {
  const state = { tasks: [], events: [{ id: 1, kind: 'reset', title: 'Historique visible', occurred_at: 1000 }] };
  await mockApi(context, state);
  let release, started;
  const pending = new Promise(resolve => { release = resolve; }), received = new Promise(resolve => { started = resolve; });
  await page.route('**/api/activity/upcoming', async route => { started(); await pending; await route.fulfill({ json: { events: [], timezone: 'Europe/Paris' } }); });
  await page.goto('/'); await page.getByRole('button', { name: 'Activité automatique' }).click(); await received;
  const dialog = page.getByRole('dialog', { name: 'Activité automatique' });
  await dialog.getByRole('tab', { name: 'Historique' }).click();
  await expect(dialog.getByText('Historique visible', { exact: true })).toBeVisible();
  const response = page.waitForResponse('**/api/activity/upcoming'); release(); await response;
  await expect(dialog.getByText('Historique visible', { exact: true })).toBeVisible();
  await expect(dialog.getByRole('tab', { name: 'Historique' })).toHaveAttribute('aria-selected', 'true');
});

test('gérer les liens actifs ou expirés et réessayer après une erreur', async ({ page, context }) => {
  const now = Math.floor(Date.now() / 1000);
  const state = { tasks: [], failShares: true, shares: [
    { id: 'active', title: 'Projet partagé', created_at: now - 500, expires_at: now + 86400, task_count: 3 },
    { id: 'expired', title: 'Ancien partage', created_at: now - 172800, expires_at: now - 60, task_count: 1 },
  ] };
  await mockApi(context, state); await page.goto('/');
  await page.getByRole('button', { name: 'Gérer les liens de partage' }).click();
  const dialog = page.getByRole('dialog', { name: 'Liens de partage' });
  await expect(dialog.getByText('Liste indisponible')).toBeVisible();
  state.failShares = false; await dialog.getByRole('button', { name: 'Réessayer' }).click();
  await expect(dialog.locator('[data-share-id="active"] .share-status')).toHaveText('Actif');
  await expect(dialog.locator('[data-share-id="expired"] .share-status')).toHaveText('Expiré');
  await expect(dialog.getByText('3 tâches dans la copie')).toBeVisible();
  page.on('dialog', pendingDialog => pendingDialog.accept());
  await dialog.locator('[data-share-id="active"]').getByRole('button', { name: 'Désactiver', exact: true }).click();
  await expect(dialog.locator('[data-share-id="active"]')).toHaveCount(0); expect(state.revoked).toBe('active');
  await dialog.locator('[data-share-id="expired"]').getByRole('button', { name: 'Supprimer', exact: true }).click();
  await expect(dialog.getByText(/Aucun lien de partage/)).toBeVisible();
});

test('conserver le formulaire de partage si une ancienne liste arrive en retard', async ({ page, context }) => {
  await mockApi(context, { tasks: [task('one', 'Une tâche')] });
  let release, started;
  const pending = new Promise(resolve => { release = resolve; }), received = new Promise(resolve => { started = resolve; });
  await page.route('**/api/shares', async route => { started(); await pending; await route.fulfill({ json: [] }); });
  await page.goto('/'); await page.getByRole('button', { name: 'Gérer les liens de partage' }).click(); await received;
  await page.getByRole('dialog', { name: 'Liens de partage' }).getByRole('button', { name: 'Fermer' }).click();
  await page.locator('[data-id="one"] .menu-toggle').click();
  await page.getByRole('menuitem', { name: 'Partager', exact: true }).click();
  const response = page.waitForResponse('**/api/shares'); release(); await response;
  const dialog = page.getByRole('dialog', { name: 'Partager la tâche' });
  await expect(dialog.getByLabel('Durée de validité')).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Créer le lien' })).toBeEnabled();
});

test('conserver la durée choisie et expliquer un lien expiré sans proposer d’import', async ({ page, context }) => {
  const state = { tasks: [task('one', 'Une tâche')] };
  await mockApi(context, state); await page.goto('/');
  await page.locator('[data-id="one"] .menu-toggle').click();
  await page.getByRole('menuitem', { name: 'Partager', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Partager la tâche' });
  await dialog.getByLabel('Durée de validité').selectOption('604800');
  await dialog.getByRole('button', { name: 'Créer le lien' }).click();
  await expect(dialog.getByLabel('Lien de partage')).toBeVisible();
  await expect(dialog.locator('.share-expiry')).toContainText('Expire le');
  expect(state.lastShareOptions).toEqual({ duration_seconds: 604800 });
  await page.route('**/api/shares/' + 's'.repeat(43), route => route.fulfill({ status: 404, json: { error: 'Lien de partage expiré ou désactivé' } }));
  await page.goto('/#partage=' + 's'.repeat(43));
  const preview = page.getByRole('dialog', { name: 'Importer les tâches' });
  await expect(preview.getByText('Lien de partage expiré ou désactivé')).toBeVisible();
  await expect(preview.getByRole('button', { name: 'Ajouter à mes tâches' })).toHaveCount(0);
});

test('centrer les pictogrammes de recherche et de partage sur mobile et ordinateur', async ({ page, context }) => {
  await mockApi(context, { tasks: [] }); await page.goto('/');
  for (const width of [320, 390, 1280]) {
    await page.setViewportSize({ width, height: 844 });
    for (const label of ['Gérer les liens de partage', 'Rechercher une tâche']) {
      const button = page.getByRole('button', { name: label });
      await expect(button).toBeVisible();
      expect(await button.evaluate(element => {
        const button = element.getBoundingClientRect(), icon = element.querySelector('svg').getBoundingClientRect();
        return Math.max(Math.abs(button.left + button.width / 2 - icon.left - icon.width / 2), Math.abs(button.top + button.height / 2 - icon.top - icon.height / 2));
      })).toBeLessThanOrEqual(0.5);
    }
  }
});
