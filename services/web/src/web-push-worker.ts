/// A push is a wake-up signal. The browser reads runner content after sign-in.
export const pushWorker = `
self.addEventListener('push', event => {
  event.waitUntil(self.registration.showNotification('Far Cooler', {
    body: 'Something needs your attention.',
    tag: 'needs-you',
  }));
});
self.addEventListener('notificationclick', event => {
  event.notification.close();
  event.waitUntil(self.clients.openWindow('/?view=needs'));
});
`
