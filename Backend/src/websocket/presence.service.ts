import { Injectable, Logger } from '@nestjs/common';
import { RedisService } from '../redis/redis.service';

@Injectable()
export class PresenceService {
  private readonly logger = new Logger(PresenceService.name);
  private readonly PRESENCE_TTL =
    parseInt(process.env.PRESENCE_TTL_SECONDS || '300', 10);
  private readonly ROOM_TTL =
    parseInt(process.env.ROOM_TTL_SECONDS || '3600', 10);
  private readonly HEARTBEAT_TTL =
    parseInt(process.env.HEARTBEAT_TTL_SECONDS || '120', 10);

  constructor(private readonly redis: RedisService) {}

  private key(...parts: string[]): string {
    return parts.join(':');
  }

  async userConnected(
    userId: string,
    socketId: string,
    correlationId?: string,
  ) {
    const socketsKey = this.key('user', userId, 'sockets');
    const onlineKey = this.key('presence:online');
    const versionKey = this.key('user', userId, 'version');
    const heartbeatKey = this.key('user', userId, 'heartbeat');

    const added = await this.redis.client.sAdd(socketsKey, socketId);
    if (added > 0) {
      await this.redis.client.expire(socketsKey, this.PRESENCE_TTL);
    }

    await this.redis.client.sAdd(onlineKey, userId);
    await this.redis.client.expire(onlineKey, this.ROOM_TTL);

    const newVersion = await this.redis.client.incr(versionKey);
    await this.redis.client.expire(versionKey, this.ROOM_TTL);

    await this.redis.client.set(
      heartbeatKey,
      Date.now().toString(),
      { EX: this.HEARTBEAT_TTL },
    );

    this.logger.debug(
      `User connected: userId=${userId} socket=${socketId} version=${newVersion} correlationId=${correlationId || 'n/a'}`,
    );

    return newVersion;
  }

  async userDisconnected(
    userId: string,
    socketId: string,
    correlationId?: string,
  ) {
    const socketsKey = this.key('user', userId, 'sockets');
    const onlineKey = this.key('presence:online');
    const versionKey = this.key('user', userId, 'version');
    const heartbeatKey = this.key('user', userId, 'heartbeat');

    await this.redis.client.sRem(socketsKey, socketId);

    const remaining = await this.redis.client.sCard(socketsKey);
    if (remaining === 0) {
      await this.redis.client.del(socketsKey);
      await this.redis.client.sRem(onlineKey, userId);

      const version = await this.redis.client.get(versionKey);
      await this.redis.client.del(versionKey);
      await this.redis.client.del(heartbeatKey);

      const roomsKey = this.key('user', userId, 'rooms');
      const rooms: string[] = await this.redis.client.sMembers(roomsKey);
      await this.redis.client.del(roomsKey);

      const pipeline = this.redis.client.multi();
      for (const roomId of rooms) {
        pipeline.sRem(this.key('room', roomId, 'users'), userId);
      }
      await pipeline.exec();

      this.logger.debug(
        `User fully disconnected: userId=${userId} version=${version} correlationId=${correlationId || 'n/a'}`,
      );
    } else {
      await this.redis.client.expire(socketsKey, this.PRESENCE_TTL);
      this.logger.debug(
        `Socket removed but user remains connected: userId=${userId} remainingSockets=${remaining}`,
      );
    }
  }

  async joinRoom(userId: string, roomId: string, correlationId?: string) {
    const userRoomsKey = this.key('user', userId, 'rooms');
    const roomUsersKey = this.key('room', roomId, 'users');

    const pipeline = this.redis.client.multi();
    pipeline.sAdd(userRoomsKey, roomId);
    pipeline.expire(userRoomsKey, this.ROOM_TTL);
    pipeline.sAdd(roomUsersKey, userId);
    pipeline.expire(roomUsersKey, this.ROOM_TTL);
    await pipeline.exec();

    this.logger.debug(
      `User joined room: userId=${userId} roomId=${roomId} correlationId=${correlationId || 'n/a'}`,
    );
  }

  async leaveRoom(userId: string, roomId: string, correlationId?: string) {
    const userRoomsKey = this.key('user', userId, 'rooms');
    const roomUsersKey = this.key('room', roomId, 'users');

    const pipeline = this.redis.client.multi();
    pipeline.sRem(userRoomsKey, roomId);
    pipeline.sRem(roomUsersKey, userId);
    await pipeline.exec();

    this.logger.debug(
      `User left room: userId=${userId} roomId=${roomId} correlationId=${correlationId || 'n/a'}`,
    );
  }

  async heartbeat(userId: string) {
    const heartbeatKey = this.key('user', userId, 'heartbeat');
    await this.redis.client.set(
      heartbeatKey,
      Date.now().toString(),
      { EX: this.HEARTBEAT_TTL },
    );
  }

  async getRoomUsers(roomId: string): Promise<string[]> {
    return this.redis.client.sMembers(this.key('room', roomId, 'users'));
  }

  async getUserRooms(userId: string): Promise<string[]> {
    return this.redis.client.sMembers(this.key('user', userId, 'rooms'));
  }

  async getOnlineUsers(): Promise<string[]> {
    return this.redis.client.sMembers('presence:online');
  }

  async getUserVersion(userId: string): Promise<string | null> {
    return this.redis.client.get(this.key('user', userId, 'version'));
  }

  async cleanupStaleUsers(maxIdleSeconds: number = 300): Promise<string[]> {
    const onlineUsers = await this.getOnlineUsers();
    const now = Date.now();
    const staleUsers: string[] = [];

    for (const userId of onlineUsers) {
      const heartbeatKey = this.key('user', userId, 'heartbeat');
      const lastSeen = await this.redis.client.get(heartbeatKey);

      if (!lastSeen || now - parseInt(lastSeen, 10) > maxIdleSeconds * 1000) {
        staleUsers.push(userId);
      }
    }

    if (staleUsers.length > 0) {
      const pipeline = this.redis.client.multi();
      for (const userId of staleUsers) {
        pipeline.sRem('presence:online', userId);
        pipeline.del(this.key('user', userId, 'sockets'));
        pipeline.del(this.key('user', userId, 'heartbeat'));
        pipeline.del(this.key('user', userId, 'version'));
        pipeline.del(this.key('user', userId, 'rooms'));
      }
      await pipeline.exec();

      this.logger.log(
        `Cleaned up ${staleUsers.length} stale users: ${staleUsers.join(', ')}`,
      );
    }

    return staleUsers;
  }
}
