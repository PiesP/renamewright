import { render } from 'solid-js/web';
import '@fontsource-variable/inter';
import '@fontsource-variable/jetbrains-mono';
import '@fontsource-variable/space-grotesk';
import { App } from './App';
import './styles/app.css';

const root = document.getElementById('root');

if (!(root instanceof HTMLElement)) {
  throw new Error('Renamewright root element is missing');
}

render(() => <App />, root);
