/* eslint-disable @typescript-eslint/explicit-module-boundary-types */

import Objection from "objection";

class Model extends Objection.Model {
	$parseDatabaseJson(json: any) {
		json = super.$parseDatabaseJson(json);
		for(const col of (this.constructor as any).dateAttributes || []) {
			if(json[col] && !(json[col] instanceof Date))
				json[col] = new Date(json[col]);
		}
		return json;
	}
}

export class User extends Model {
	static tableName = "users";

	id!: number;
	email!: string;

	sessions?: Session[];
	authorizations?: Authorization[];

	static get relationMappings() {
		return {
			sessions: { relation: Model.HasManyRelation, modelClass: Session, join: { from: "users.id", to: "sessions.userId" }},
			authorizations: { relation: Model.HasManyRelation, modelClass: Authorization, join: { from: "users.id", to: "authorizations.userId" }},
		};
	}
}

export class Origin extends Model {
	static tableName = "origins";

	id!: number;
	origin!: string;

	authorizations?: Authorization[];

	static get relationMappings() {
		return {
			authorizations: { relation: Model.HasManyRelation, modelClass: Authorization, join: { from: "origins.id", to: "authorizations.originId" }},
		};
	}
}

export class Authorization extends Model {
	static tableName = "authorizations";

	id!: number;
	userId!: number;
	originId!: number;

	user?: User;
	origin?: Origin;

	static get relationMappings() {
		return {
			user: { relation: Model.BelongsToOneRelation, modelClass: User, join: { from: "authorizations.userId", to: "users.id" }},
			origin: { relation: Model.BelongsToOneRelation, modelClass: Origin, join: { from: "authorizations.originId", to: "origins.id" }},
		};
	}
}

export class Session extends Model {
	static tableName = "sessions";
	static dateAttributes = ["expirationDate"];

	id!: string;
	userId!: number | null;
	expirationDate!: Date | null;

	user?: User | null;

	static get relationMappings() {
		return {
			user: { relation: Model.BelongsToOneRelation, modelClass: User, join: { from: "sessions.userId", to: "users.id" }},
		};
	}
}
