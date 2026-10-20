import assert from 'node:assert/strict';

const shortHelp='Explicação sintética: o saldo em conta é separado dos proventos acumulados.';
const longHelp=('Definição longa para verificar a leitura sem cortar informação. Esta explicação usa apenas valores e nomes fictícios. ').repeat(12)+'Fonte: '+('exemplo-sem-espacos-').repeat(24);

/** Render the real shared components, without IPC, local financial data or application fixtures. */
export function tooltipFixtureSource(){return `
import React from '/node_modules/.vite/deps/react.js';
import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import {Def,Field,Modal} from '/src/components.tsx';
import '/src/styles/tokens.css';
import '/src/styles/base.css';
import '/src/styles/views.css';
const h=React.createElement;
function Fixture(){
  const [dialog,setDialog]=React.useState(false);
  return h('main',{id:'tooltip-fixture',style:{minHeight:'160vh',padding:'24px'}},
    h('button',{id:'open-tooltip-dialog',type:'button',onClick:()=>setDialog(true)},'Abrir diálogo de teste'),
    h('div',{id:'short-anchor',style:{marginTop:'70px'}},h(Def,{text:${JSON.stringify(shortHelp)}},'Ajuda breve')),
    h('div',{id:'long-anchor',style:{position:'fixed',right:'12px',bottom:'12px'}},h(Def,{text:${JSON.stringify(longHelp)}},'Definição extensa')),
    h('details',{className:'extra-details',style:{marginTop:'60px'}},h('summary',null,'Informações adicionais'),h('p',null,'Conteúdo secundário fictício.')),
    dialog&&h(Modal,{title:'Diálogo de tooltip',onClose:()=>setDialog(false)},h('form',null,
      h(Field,{label:'Campo de teste',hint:${JSON.stringify(shortHelp)}},h('input',{autoFocus:true,name:'fixture-input'})),
      h('div',{id:'dialog-anchor',style:{marginTop:'24px'}},h(Def,{text:${JSON.stringify(longHelp)}},'Ajuda dentro do diálogo')),
      h('button',{type:'button',onClick:()=>setDialog(false)},'Fechar diálogo de teste')
    ))
  );
}
ReactDOM.createRoot(document.getElementById('root')).render(h(Fixture));
`;}

export async function verifyTooltips({browser,base,check,shot,browserErrors}){
  const tooltip=page=>page.getByRole('tooltip');
  const closed=page=>tooltip(page).waitFor({state:'hidden'});
  async function contained(page){
    const box=await tooltip(page).boundingBox();
    const viewport=page.viewportSize();
    assert.ok(box&&box.width<=321&&box.x>=11&&box.y>=11&&box.x+box.width<=viewport.width-11&&box.y+box.height<=viewport.height-11,`Tooltip clipped: ${JSON.stringify({box,viewport})}`);
    assert.ok(box.height<=viewport.height*.8+1,'Long tooltip must keep a scrollable viewport limit');
    const hit=await tooltip(page).evaluate(element=>{
      const box=element.getBoundingClientRect();
      return element.contains(document.elementFromPoint(box.x+Math.min(20,box.width/2),box.y+Math.min(20,box.height/2)));
    });
    assert.equal(hit,true,'Tooltip should be visible above clipping containers and native dialogs');
  }
  async function association(page,anchor){
    await tooltip(page).waitFor();
    assert.equal(await tooltip(page).count(),1);
    assert.equal(await anchor.getAttribute('aria-expanded'),'true');
    const id=await tooltip(page).getAttribute('id');
    assert.ok((await anchor.getAttribute('aria-describedby'))?.split(/\s+/).includes(id),'Focused label should describe its visible tooltip');
    assert.equal(await anchor.getAttribute('title'),null,'No native title tooltip should compete with the visible help');
  }
  async function open(viewport,touch=false){
    const context=await browser.newContext({viewport,hasTouch:touch,reducedMotion:'reduce'});
    const page=await context.newPage();
    page.setDefaultTimeout(12000);
    page.on('pageerror',error=>browserErrors.push({mode:'tooltip',message:error.message}));
    await page.route('**/src/main.tsx*',route=>route.fulfill({status:200,contentType:'application/javascript',body:tooltipFixtureSource()}));
    await page.goto(base,{waitUntil:'networkidle',timeout:60000});
    await page.locator('#tooltip-fixture').waitFor();
    return {context,page};
  }
  for(const width of [1000,760]){
    const {context,page}=await open({width,height:760});
    const brief=page.locator('#short-anchor .def');
    const long=page.locator('#long-anchor .def');
    assert.equal(await page.locator('details.extra-details').evaluate(element=>element.open),false);
    await brief.hover();await association(page,brief);assert.equal(await tooltip(page).innerText(),shortHelp);await contained(page);
    await tooltip(page).hover();await page.waitForTimeout(160);assert.equal(await tooltip(page).isVisible(),true,'Mouse can move from trigger to tooltip');
    await page.mouse.move(2,2);await closed(page);
    await brief.focus();await association(page,brief);
    await page.keyboard.press('Escape');await closed(page);assert.equal(await brief.evaluate(element=>element===document.activeElement),true);
    await page.keyboard.press('Enter');await association(page,brief);
    await page.keyboard.press(' ');await closed(page);
    await long.hover();await association(page,long);assert.equal(await tooltip(page).innerText(),longHelp);await contained(page);
    await shot(page,'tooltip-long-'+width,{fullPage:false,preserveFocus:true});
    await page.setViewportSize({width:width-40,height:700});
    // setViewportSize acknowledges the new metrics before Chromium dispatches resize.
    // Let that event reposition the popover and commit its layout before measuring.
    await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    await contained(page);
    await page.keyboard.press('Escape');await closed(page);await page.mouse.move(2,2);
    await page.locator('#open-tooltip-dialog').click();
    const dialog=page.getByRole('dialog',{name:'Diálogo de tooltip',exact:true});await dialog.waitFor();
    const input=dialog.getByLabel('Campo de teste',{exact:true});
    await input.waitFor();
    const hint=dialog.locator('.field .def');await hint.focus();await association(page,hint);
    assert.equal(await input.getAttribute('name'),'fixture-input');
    assert.equal(await tooltip(page).evaluate(element=>!!element.closest('dialog[open]')),true,'Dialog help remains within the modal active scope');
    await contained(page);await page.keyboard.press('Escape');await closed(page);
    assert.equal(await dialog.evaluate(element=>element.open),true,'Escape closes help before the containing dialog');
    await dialog.locator('#dialog-anchor .def').hover();await contained(page);
    await shot(page,'tooltip-dialog-'+width,{fullPage:false,preserveFocus:true});
    await page.keyboard.press('Escape');await closed(page);await dialog.getByRole('button',{name:'Fechar diálogo de teste',exact:true}).click();await dialog.waitFor({state:'detached'});
    assert.equal(await tooltip(page).count(),0);
    check('tooltip-mouse-keyboard-Escape-dialog-longtext-'+width);
    await context.close();
  }
  const {context,page}=await open({width:760,height:760},true);
  const brief=page.locator('#short-anchor .def');
  await brief.tap();await association(page,brief);await contained(page);
  await brief.tap();await closed(page);
  await brief.tap();await association(page,brief);
  await page.locator('details.extra-details > summary').tap();await closed(page);
  assert.equal(await page.locator('details.extra-details').evaluate(element=>element.open),true);
  check('tooltip-touch-tap-toggle-and-outside-dismiss');
  await context.close();
}
