import { describe, expect, it } from 'vitest';
import { notificationText } from './notification-text';

describe('notificationText', () => {
  it('names each known kind', () => {
    expect(notificationText('friend_request', 'Alice')).toBe('@Alice sent you a friend request');
    expect(notificationText('friend_accepted', 'Alice')).toBe(
      '@Alice accepted your friend request',
    );
    expect(notificationText('salute', 'Alice')).toBe('@Alice saluted your profile. o7');
  });

  it('never calls an unknown kind an accepted friend request', () => {
    expect(notificationText('lfg_join_request', 'Alice')).toBe('New activity from @Alice');
  });

  it('copes without an actor', () => {
    expect(notificationText('salute', null)).toBe('@Someone saluted your profile. o7');
  });
});
