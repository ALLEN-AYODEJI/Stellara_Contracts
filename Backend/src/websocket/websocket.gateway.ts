import {
  WebSocketGateway,
  WebSocketServer,
  OnGatewayConnection,
  OnGatewayDisconnect,
  SubscribeMessage,
} from '@nestjs/websockets';
import { Logger } from '@nestjs/common';
import { Server, Socket } from 'socket.io';
import { randomUUID } from 'crypto';
import { PresenceService } from './presence.service';
import { MetricsService } from '../observability/services/metrics.service';
import { TracingService } from '../observability/services/tracing.service';

@WebSocketGateway({
  cors: { origin: '*' },
  transports: ['websocket', 'polling'],
})
export class WebsocketGateway
  implements OnGatewayConnection, OnGatewayDisconnect
{
  @WebSocketServer()
  server: Server;

  private readonly logger = new Logger(WebsocketGateway.name);
  private disconnectingClients = new Set<string>();

  constructor(
    private readonly presenceService: PresenceService,
    private readonly metricsService: MetricsService,
    private readonly tracingService: TracingService,
  ) {}

  async handleConnection(client: Socket) {
    const userId = client.handshake.auth.userId;
    if (!userId) {
      this.logger.warn(
        `WS connection rejected: missing userId from ${client.id}`,
      );
      client.emit('auth:error', { message: 'userId is required' });
      client.disconnect();
      return;
    }

    const correlationId = client.handshake.auth.correlationId || randomUUID();
    const namespace = (client.nsp as any)?.name || '/';

    this.metricsService.recordWebSocketConnection(namespace, correlationId);

    const traceContext = this.tracingService.createTraceContext(
      undefined,
      undefined,
      userId,
      { correlationId, socketId: client.id, type: 'connection' },
    );

    const version = await this.presenceService.userConnected(
      userId,
      client.id,
      traceContext.traceId,
    );

    const rooms = await this.presenceService.getUserRooms(userId);
    const joinedRooms: string[] = [];

    for (const roomId of rooms) {
      try {
        client.join(roomId);
        joinedRooms.push(roomId);
      } catch {
        this.metricsService.recordWebSocketMessage(
          namespace,
          'recover_room_error',
          correlationId,
        );
      }
    }

    if (joinedRooms.length > 0) {
      this.server.to(userId).emit('presence:room_recovery', {
        userId,
        rooms: joinedRooms,
        version,
        correlationId: traceContext.traceId,
        timestamp: Date.now(),
      });
    }

    this.server.to(userId).emit('presence:update', {
      userId,
      status: 'online',
      socketId: client.id,
      version,
      rooms: joinedRooms,
      correlationId: traceContext.traceId,
      timestamp: Date.now(),
    });

    this.logger.log(
      `WS connection: userId=${userId} socket=${client.id} version=${version} rooms=${JSON.stringify(joinedRooms)}`,
    );
  }

  async handleDisconnect(client: Socket) {
    const userId = client.handshake.auth.userId;
    if (!userId) return;

    if (this.disconnectingClients.has(client.id)) {
      this.disconnectingClients.delete(client.id);
      return;
    }

    const correlationId = client.handshake.auth.correlationId || randomUUID();
    const namespace = (client.nsp as any)?.name || '/';

    this.metricsService.recordWebSocketDisconnection(
      namespace,
      'client_disconnect',
      correlationId,
    );

    await this.presenceService.userDisconnected(
      userId,
      client.id,
      correlationId,
    );

    this.server.to(userId).emit('presence:update', {
      userId,
      status: 'offline',
      socketId: client.id,
      correlationId,
      timestamp: Date.now(),
    });
  }

  @SubscribeMessage('join-room')
  async joinRoom(client: Socket, roomId: string) {
    const userId = client.handshake.auth.userId;
    if (!userId) {
      client.emit('auth:error', { message: 'userId is required' });
      return;
    }

    const correlationId = randomUUID();
    const namespace = (client.nsp as any)?.name || '/';

    this.metricsService.recordWebSocketMessage(namespace, 'join_room', correlationId);

    await this.presenceService.joinRoom(userId, roomId, correlationId);
    void client.join(roomId);

    await this.presenceService.heartbeat(userId);

    const users = await this.presenceService.getRoomUsers(roomId);
    this.server.to(roomId).emit('presence:update', {
      roomId,
      users,
      correlationId,
      timestamp: Date.now(),
      event: 'join',
      userId,
    });
  }

  @SubscribeMessage('leave-room')
  async leaveRoom(client: Socket, roomId: string) {
    const userId = client.handshake.auth.userId;
    if (!userId) {
      client.emit('auth:error', { message: 'userId is required' });
      return;
    }

    const correlationId = randomUUID();
    const namespace = (client.nsp as any)?.name || '/';

    this.metricsService.recordWebSocketMessage(namespace, 'leave_room', correlationId);

    await this.presenceService.leaveRoom(userId, roomId, correlationId);
    void client.leave(roomId);

    await this.presenceService.heartbeat(userId);

    const users = await this.presenceService.getRoomUsers(roomId);
    this.server.to(roomId).emit('presence:update', {
      roomId,
      users,
      correlationId,
      timestamp: Date.now(),
      event: 'leave',
      userId,
    });
  }

  @SubscribeMessage('presence:heartbeat')
  async handleHeartbeat(client: Socket) {
    const userId = client.handshake.auth.userId;
    if (!userId) return;

    await this.presenceService.heartbeat(userId);
    client.emit('presence:pong', { timestamp: Date.now() });
  }

  @SubscribeMessage('message')
  handleMessage(
    client: Socket,
    payload: { roomId: string; message: string },
  ) {
    const userId = client.handshake.auth.userId;
    if (!userId) {
      client.emit('auth:error', { message: 'userId is required' });
      return;
    }

    const correlationId = randomUUID();
    const namespace = (client.nsp as any)?.name || '/';

    this.metricsService.recordWebSocketMessage(namespace, 'message', correlationId);

    const traceContext = this.tracingService.createTraceContext(
      undefined,
      undefined,
      userId,
      { correlationId, roomId: payload.roomId },
    );

    void this.presenceService.heartbeat(userId);

    this.server.to(payload.roomId).emit('message', {
      ...payload,
      userId,
      correlationId: traceContext.traceId,
      timestamp: Date.now(),
    });
  }
}
