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
      return json(200, { token: 's'.repeat(43) });
    }
    if (url.pathname === `/api/shares/${'s'.repeat(43)}`) {
      if (request.method() === 'GET') return json(200, state.snapshot);
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
    if (url.pathname === '/api/activity') return json(200, []);
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
    await sender.getByRole('button', { name: 'Partager', exact: true }).click();
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
